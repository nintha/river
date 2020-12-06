#[macro_use]
extern crate num_derive;

use crate::rtmp_server::accept_loop;
use crate::util::spawn_and_log_error;

mod eventbus;
mod http_flv;
mod protocol;
mod rtmp_server;
mod util;
mod ws_h264;

fn main() -> anyhow::Result<()> {
    util::init_logger();

    spawn_and_log_error(http_flv::run_server("0.0.0.0:8080"));
    spawn_and_log_error(ws_h264::run_server("0.0.0.0:8081"));
    let server = "0.0.0.0:11935";
    smol::block_on(accept_loop(server))
}
