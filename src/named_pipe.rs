use std::str::FromStr;
use std::thread;
use std::time;
use std::io::{Write, Read};

use mio::*;
use mio::tcp::*;
use mio_named_pipes::NamedPipe;

use std::fs::OpenOptions;
use std::io::prelude::*;
use std::os::windows::fs::*;
use std::os::windows::io::*;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {}", stringify!($e), e),
    })
}

fn sleep_ms(millis: u64) {
    thread::sleep(time::Duration::from_millis(millis))
}

const SERVER_STREAM: Token = Token(0);
const CLIENT_STREAM: Token = Token(1);

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
    named_pipe: NamedPipe,
    state: StreamState
}

impl TestStream {
    fn new_server() -> TestStream {
        TestStream {
            token: SERVER_STREAM,
            stream: NamedPipe::new(r"\\.\pipe\test-pipe").unwrap(),
            state: StreamState::Initial
        }
    }

    fn new_client() -> TestStream {
        let mut opts = OpenOptions::new();
        opts.read(true)
            .write(true)
            .custom_flags(winapi::FILE_FLAG_OVERLAPPED);
        let file = opts.open(r"\\.\pipe\test-pipe").unwrap();
        let pipe = unsafe { NamedPipe::from_raw_handle(file.into_raw_handle()) };
        TestStream {
            token: CLIENT_STREAM,
            named_pipe: pipe,
            state: StreamState::Initial
        }
    }

    fn change_state(&mut self, new_state: StreamState) {
        info!("stream {:?} changed from {:?} to {:?}", self.token, self.state, new_state);

        self.state = new_state;
    }

    fn start(&mut self, poll: &Poll) {
        poll.register(&self.stream, self.token, Ready::writable(), PollOpt::level()).unwrap();
        if self.token == SERVER_STREAM {
            let _ = self.named_pipe.connect();
        }
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
    let mut events = Events::with_capacity(1024);

    let server = TestStream::new_server();
    server.start(&poll);

    sleep_ms(100);

    let mut client = TestStream::new_client();
    client.start(&poll);

    loop {
        let event_count = poll.poll(&mut events, None).unwrap();
        info!("poll raised {} events", event_count);

        for event in events.iter() {
            match event.token() {
                CLIENT_STREAM => {
                    info!("client stream event: {:?} ", event.kind());
                    client.process(&poll, event.kind());
                },
                SERVER_STREAM => {
                    info!("server stream event: {:?} ", event.kind());
                    server.process(&poll, event.kind());
                }
                _ => unreachable!(),
            }

            if server.state == StreamState::Dead && client.state == StreamState::Dead {
                return;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use env_logger;

    use mio::*;
    use std::time::Duration;

    #[test]
    fn client_named_pipe_should_become_readable() {
        let _ = env_logger::init();
        info!("Logger initialized");

        super::scenario();
    }

    #[test]
    fn when_registering_named_pipe_server_writable_is_polled() {
        let _ = env_logger::init();
        let poll = t!(Poll::new());

        let (server, name) = super::server(1);
        t!(poll.register(&server, Token(0), Ready::writable(), PollOpt::edge()));

        let _ = server.connect(); // this one seems to be required ?!

        let client = super::client(&name);
        t!(poll.register(&client, Token(1), Ready::writable(), PollOpt::edge()));

        let mut events = Events::with_capacity(128);

        loop {
            t!(poll.poll(&mut events, None));

            let raised_events = events.iter().collect::<Vec<_>>();
            debug!("events {:?}", raised_events);
            if raised_events.iter().any(|e| { e.token() == Token(0) && e.kind().is_writable() }) {
                break;
            }
        }
   }
}
