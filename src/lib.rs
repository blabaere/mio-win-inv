#[macro_use] 
extern crate log;
extern crate env_logger;
extern crate mio;
//extern crate bytes;
//extern crate slab;

//use mio;
//use mio::tcp;

use std::collections::HashMap;
use std::sync::mpsc;
use std::io;
use std::net::SocketAddr;

// Talks to the session via the event loop and the channel
// Lives in the main/test thread.
pub struct Controller {
    cmd_sender: mio::Sender<Command>,
    evt_receiver: mpsc::Receiver<io::Result<()>>,
}

// Receives commands and readiness notifications via the event loop
// Responsible for dispatching all these, creating and storing Server and Client
// Lives in the event loop thread.
pub struct Session {
    token_generator: usize,
    evt_sender: mpsc::Sender<io::Result<()>>,
    servers: HashMap<mio::Token, Server>,
    clients: HashMap<mio::Token, Client>
}

pub enum Command {
    Connect(SocketAddr),
    Listen(SocketAddr),
    Send,
    Recv,
    Shutdown
}

// wraps a tcp stream, performs handshake, send and recv messages
// according to its state and received commands.
struct ProtoStream {
    token: mio::Token,
    stream: mio::tcp::TcpStream
}

enum ProtoStreamState {
    Initial
}

// wraps a tcp listener, creates and stores ProtoStream 
struct Server {
    token: mio::Token,
    listener: mio::tcp::TcpListener
}

// wraps a ProtoStream
struct Client {
    stream: ProtoStream
}

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

    pub fn listen(&self, addr: SocketAddr) -> Result<(), io::Error> {
        self.send_cmd(Command::Listen(addr)).and_then(|_| self.recv_evt())
    }

    pub fn connect(&self, addr: SocketAddr) -> Result<(), io::Error> {
        self.send_cmd(Command::Connect(addr)).and_then(|_| self.recv_evt())
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
            token_generator: 0,
            evt_sender: evt_tx,
            servers: HashMap::new(),
            clients: HashMap::new()
        }
    }

    fn next_token(&mut self) -> mio::Token {
        self.token_generator += 1;

        mio::Token(self.token_generator)
    }

    fn send_event(&mut self, evt: io::Result<()>) {
        let _ = self.evt_sender.send(evt);
    }

    fn send_ok_event(&mut self) {
        let evt = Ok(());
        self.send_event(evt);
    }

    fn connect(&mut self, event_loop: &mut mio::EventLoop<Session>, addr: SocketAddr) {
        let token = self.next_token();
        let client = Client::new(token, &addr);

        client.register(event_loop);
        self.clients.insert(token, client);

        self.send_ok_event();
    }

    fn listen(&mut self, event_loop: &mut mio::EventLoop<Session>, addr: SocketAddr) {
        let token = self.next_token();
        let server = Server::new(token, &addr);

        server.register(event_loop);
        self.servers.insert(token, server);

        self.send_ok_event();
    }

    fn get_server<'a>(&'a mut self, tok: &mio::Token) -> Option<&'a mut Server> {
        self.servers.get_mut(tok)
    }

    fn accept(&mut self, tok: &mio::Token) -> Option<mio::tcp::TcpStream> {
        self.get_server(tok).map(|s| s.accept().unwrap())
    }

    fn on_server_ready(&mut self, event_loop: &mut mio::EventLoop<Session>, tok: mio::Token, events: mio::EventSet) {
        info!("on_server_ready {:?} {:?}", tok, events);
        let stream = self.accept(&tok).unwrap();
        let client_tok = self.next_token();
        let client = Client::accepted(client_tok, stream);

        client.register(event_loop);
        self.clients.insert(client_tok, client);

        self.send_ok_event();

        info!("Client/Server link is setup: receiver is {:?}", client_tok);
    }
}

impl mio::Handler for Session {
    type Timeout = usize;
    type Message = Command;

    fn ready(&mut self, event_loop: &mut mio::EventLoop<Session>, tok: mio::Token, events: mio::EventSet) {
        info!("ready {:?} {:?}", tok, events);

        if self.servers.contains_key(&tok) {
            return self.on_server_ready(event_loop, tok, events);
        }

        /*if let Some(server) = self.servers.get_mut(&tok) {
            return server.ready(event_loop, events);
        }

        if let Some(client) = self.clients.get_mut(&tok) {
            return client.ready(event_loop, events);
        }*/
    }
    fn notify(&mut self, event_loop: &mut mio::EventLoop<Session>, cmd: Command) {
        match cmd {
            Command::Shutdown => event_loop.shutdown(),
            Command::Connect(addr) => self.connect(event_loop, addr),
            Command::Listen(addr) => self.listen(event_loop, addr),
            _ => {}
        }
    }
}

impl Server {
    fn new(tok: mio::Token, addr: &SocketAddr) -> Server {
        Server {
            token: tok,
            listener: mio::tcp::TcpListener::bind(addr).unwrap()
        }
    }

    fn register(&self, event_loop: &mut mio::EventLoop<Session>) {
        let interest = mio::EventSet::readable();
        let poll_opt = mio::PollOpt::edge();

        event_loop.register(&self.listener, self.token, interest, poll_opt).unwrap();
    }

    fn accept(&mut self) -> io::Result<mio::tcp::TcpStream> {
        match try!(self.listener.accept()) {
            Some((stream, _)) => Ok(stream),
            None              => Err(other_io_error("acceptor ready but would block"))
        }
    }
}

impl Client {
    fn new(tok: mio::Token, addr: &SocketAddr) -> Client {
        Client {
            stream: ProtoStream::new(tok, addr)
        }
    }

    fn accepted(tok: mio::Token, stream: mio::tcp::TcpStream) -> Client {
        Client {
            stream: ProtoStream::accepted(tok, stream)
        }
    }

    fn register(&self, event_loop: &mut mio::EventLoop<Session>) {
        self.stream.register(event_loop);
    }

    fn ready(&mut self, event_loop: &mut mio::EventLoop<Session>, events: mio::EventSet) {
    }
}

impl ProtoStream {
    fn new(tok: mio::Token, addr: &SocketAddr) -> ProtoStream {
        ProtoStream {
            token: tok,
            stream: mio::tcp::TcpStream::connect(addr).unwrap()
        }
    }

    fn accepted(tok: mio::Token, stream: mio::tcp::TcpStream) -> ProtoStream {
        ProtoStream {
            token: tok,
            stream: stream
        }
    }

    fn register(&self, event_loop: &mut mio::EventLoop<Session>) {
        let interest = mio::EventSet::all();
        let poll_opt = mio::PollOpt::edge();

        event_loop.register(&self.stream, self.token, interest, poll_opt).unwrap();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use env_logger;

    use mio;

    use std::sync::mpsc;
    use std::io;
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::thread;

    fn localhost() -> SocketAddr {
        FromStr::from_str("127.0.0.1:18080").unwrap()
    }

    #[test]
    fn it_works() {
        let _ = env_logger::init();
        info!("Logging initialized");

        let mut event_loop = mio::EventLoop::new().unwrap();
        let (tx, rx) = mpsc::channel();
        let ctrl = Controller::new(event_loop.channel(), rx);

        let el_thread = thread::spawn(move || run_event_loop(&mut event_loop, tx));
        let addr = localhost();

        ctrl.listen(addr).unwrap();
        ctrl.connect(addr).unwrap();

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
