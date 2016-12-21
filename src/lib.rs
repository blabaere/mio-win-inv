#[macro_use] 
extern crate log;
extern crate env_logger;
extern crate mio;

#[cfg(windows)] extern crate mio_named_pipes;
#[cfg(windows)] extern crate winapi;

pub mod tcp;
#[cfg(windows)]
pub mod named_pipes;
