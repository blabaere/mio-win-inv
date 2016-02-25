#[macro_use] 
extern crate log;
extern crate env_logger;
extern crate mio;
//extern crate bytes;
//extern crate slab;

//use mio;
//use mio::tcp;

use std::sync::mpsc;
use std::io;
use std::net::SocketAddr;
use std::str::FromStr;

// Talks to the session via the event loop and the channel
// Lives in the main/test thread.
pub struct Controller {
    cmd_sender: mio::Sender<Command>,
    evt_receiver: mpsc::Receiver<io::Result<()>>,
}

// Receives commands and readyness notifications via the event loop
// Responsible for dispatching all these, creating and and storing Server and Client
// Lives in the event loop  thread.
pub struct Session {
    evt_sender: mpsc::Sender<io::Result<()>>
}

pub enum Command {
    Connect,
    Listen,
    Send,
    Recv,
    Shutdown
}

// wraps a tcp stream, performs handshake, send and recv messages
// according to its state and received commands.
struct ProtoStream;

enum ProtoStreamState {
    Initial
}


// wraps a tcp listener, creates and stores ProtoStream 
struct Server;

// wraps a ProtoStream
struct Client;

pub fn other_io_error(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg)
}

pub fn convert_notify_err<T>(err: mio::NotifyError<T>) -> io::Error {
    match err {
        mio::NotifyError::Io(e)     => e,
        mio::NotifyError::Closed(_) => other_io_error("cmd channel closed"),
        mio::NotifyError::Full(_)   => io::Error::new(io::ErrorKind::WouldBlock, "cmd channel full")
    }
}

impl Controller {
    pub fn new(cmd_tx: mio::Sender<Command>, evt_rx: mpsc::Receiver<io::Result<()>>) -> Controller {
        Controller {
            cmd_sender: cmd_tx,
            evt_receiver: evt_rx
        }
    }

    fn send_cmd(&self, cmd: Command) -> Result<(), io::Error> {
        self.cmd_sender.send(cmd).map_err(|e| convert_notify_err(e))
    }

    fn recv_evt(&self) -> Result<(), io::Error> {
        match self.evt_receiver.recv() {
            Ok(_)  => Ok(()),
            Err(_) => Err(other_io_error("evt channel closed"))
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.send_cmd(Command::Shutdown);
    }
}

impl Session {
    fn new(evt_tx: mpsc::Sender<io::Result<()>>) -> Session {
        Session {
            evt_sender: evt_tx
        }
    }
}

impl mio::Handler for Session {
    type Timeout = usize;
    type Message = Command;

    fn ready(&mut self, event_loop: &mut mio::EventLoop<Session>, token: mio::Token, events: mio::EventSet) {
    }
    fn notify(&mut self, event_loop: &mut mio::EventLoop<Session>, cmd: Command) {
        match cmd {
            Command::Shutdown => event_loop.shutdown(),
            _ => {},
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::sync::mpsc;
    use std::io;
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::thread;

    #[test]
    fn it_works() {
        let _ = env_logger::init();
        info!("Logging initialized");

        let mut event_loop = mio::EventLoop::new().unwrap();
        let (tx, rx) = mpsc::channel();
        let ctrl = Controller::new(event_loop.channel(), rx);

        let el_thread = thread::spawn(move || run_event_loop(&mut event_loop, tx));

        drop(ctrl);
        el_thread.join().unwrap();
    }

    fn run_event_loop(
        event_loop: &mut mio::EventLoop<Session>, 
        evt_tx: mpsc::Sender<io::Result<()>>) {

        let mut handler = Session::new(evt_tx);
        let exec = event_loop.run(&mut handler);

        match exec {
            Ok(_) => debug!("event loop exited"),
            Err(e) => error!("event loop failed to run: {}", e)
        }
    }
}
