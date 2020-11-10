#[macro_use]
extern crate num_derive;

use crate::http_flv::run_server;
use crate::rtmp_server::accept_loop;
use crate::util::spawn_and_log_error;

mod eventbus;
mod http_flv;
mod protocol;
mod rtmp_server;
mod util;

fn main() -> anyhow::Result<()> {
    util::init_logger();

    spawn_and_log_error(run_server("0.0.0.0:8080"));
    let server = "0.0.0.0:11935";
    smol::block_on(accept_loop(server))
}
