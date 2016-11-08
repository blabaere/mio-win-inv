#[macro_use] 
extern crate log;
extern crate env_logger;
extern crate mio;

use std::net::SocketAddr;
use std::str::FromStr;
use std::thread;
use std::time;
use std::io::{Write, Read};

use mio::*;
use mio::tcp::*;

fn localhost() -> SocketAddr {
    FromStr::from_str("127.0.0.1:18080").unwrap()
}

fn sleep_ms(millis: u64) {
    thread::sleep(time::Duration::from_millis(millis))
}

const SERVER_LISTENER: Token = Token(0);
const CLIENT_STREAM: Token = Token(1);
const SERVER_STREAM: Token = Token(2);

struct TestListener {
    token: Token,
    listener: TcpListener
}

impl TestListener {
    fn bind(addr: &SocketAddr, tok: Token) -> TestListener {
        TestListener {
            token: tok,
            listener: TcpListener::bind(&addr).unwrap()
        }
    }

    fn start(&self, poll: &Poll) {
        poll.register(&self.listener, self.token, Ready::readable(), PollOpt::edge()).unwrap();
    }

    fn accept(&self, tok: Token) -> TestStream {
        let (s, _) = self.listener.accept().unwrap();
        TestStream {
            token: tok,
            stream: s,
            state: StreamState::Initial
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
enum StreamState {
    Initial,
    HandshakeRx,
    HandshakeTx,
    Active,
    Dead
}

struct TestStream {
    token: Token,
    stream: TcpStream,
    state: StreamState
}

impl TestStream {
    fn connect(addr: &SocketAddr, tok: Token) -> TestStream {
        TestStream {
            token: tok,
            stream: TcpStream::connect(&addr).unwrap(),
            state: StreamState::Initial
        }
    }

    fn change_state(&mut self, new_state: StreamState) {
        info!("stream state changed from {:?} to {:?}", self.state, new_state);

        self.state = new_state;
    }

    fn start(&mut self, poll: &Poll) {
        poll.register(&self.stream, self.token, Ready::writable(), PollOpt::level()).unwrap();
        self.change_state(StreamState::HandshakeTx);
    }

    fn process(&mut self, poll: &Poll, evt: Ready) {
        match self.state {
            StreamState::HandshakeTx => {
                assert!(evt.is_writable());
                assert!(!evt.is_readable());
                let buffer: [u8; 8] = [6; 8];
                let written = self.stream.write(&buffer).unwrap();
                assert_eq!(8, written);
                poll.reregister(&self.stream, self.token, Ready::readable(), PollOpt::level()).unwrap();
                self.change_state(StreamState::HandshakeRx);
            },
            StreamState::HandshakeRx => {
                assert!(!evt.is_writable());
                assert!(evt.is_readable());
                let mut buffer: [u8; 8] = [0; 8];
                let read = self.stream.read(&mut buffer).unwrap();
                assert_eq!(8, read);
                poll.reregister(&self.stream, self.token, Ready::all(), PollOpt::edge()).unwrap();
                self.change_state(StreamState::Active);
            },
            StreamState::Active => {
                assert!(evt.is_writable());
                assert!(!evt.is_readable());
                assert!(!evt.is_error());
                assert!(!evt.is_hup());
                self.change_state(StreamState::Dead);
            }
            _ => unreachable!()
        }
    }
}

pub fn scenario() {
    let poll = Poll::new().unwrap();
    let addr = localhost();
    let mut events = Events::with_capacity(1024);

    let listener = TestListener::bind(&addr, SERVER_LISTENER);
    listener.start(&poll);

    sleep_ms(500);

    let mut client = TestStream::connect(&addr, CLIENT_STREAM);
    client.start(&poll);

    let mut maybe_server = None;

    loop {
        let event_count = poll.poll(&mut events, None).unwrap();
        info!("poll raised {} events", event_count);

        for event in events.iter() {
            match event.token() {
                SERVER_LISTENER => {
                    info!("server listener event: {:?} ", event.kind());
                    assert!(maybe_server.is_none());
                    let mut server = listener.accept(SERVER_STREAM);

                    server.start(&poll);
                    maybe_server = Some(server);
                },
                CLIENT_STREAM => {
                    info!("client stream event: {:?} ", event.kind());
                    client.process(&poll, event.kind());
                },
                SERVER_STREAM => {
                    info!("server stream event: {:?} ", event.kind());
                    let mut server = maybe_server.take().unwrap();
                    server.process(&poll, event.kind());
                    maybe_server = Some(server)
                }
                _ => unreachable!(),
            }

            if let Some(server) = maybe_server.as_ref() {
                if server.state == StreamState::Dead && client.state == StreamState::Dead {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use env_logger;

    use mio::*;

    #[test]
    fn what_happens_when_binding_twice_on_the_sameport_with_mio() {
        let addr = "127.0.0.1:5455".parse().unwrap();
        let listener1 = tcp::TcpListener::bind(&addr);
        assert!(listener1.is_ok());

        super::sleep_ms(500);
        let listener2 = tcp::TcpListener::bind(&addr);
        assert!(listener2.is_err());
    }

    #[test]
    fn what_happens_when_binding_twice_on_the_sameport_with_std() {
        let listener1 = ::std::net::TcpListener::bind("127.0.0.1:5456");
        assert!(listener1.is_ok());

        super::sleep_ms(500);
        let listener2 = ::std::net::TcpListener::bind("127.0.0.1:5456");
        assert!(listener2.is_err());
    }

    #[test]
    fn spurious_readable_with_mio_061_but_not_with_060() {
        let _ = env_logger::init();
        info!("Logger initialized");

        super::scenario();
    }
}
