//! Hardware-free transport contracts. Socket I/O and pacing belong on workers,
//! never in an audio callback. This prototype is not an authenticated live host.
mod buffer;
mod control;
mod packet;
pub use buffer::*;
pub use control::*;
pub use packet::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Format,
    Size,
    Sample,
    Identity,
    Timeline,
    Late,
    Future,
    Duplicate,
    Capacity,
}
mod handoff;
pub use handoff::*;
mod network;
pub use network::*;
mod resources;
pub use resources::*;
