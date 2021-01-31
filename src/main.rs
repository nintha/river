#[macro_use]
extern crate num_derive;

use clap::crate_version;
use clap::Clap;
use crate::rtmp_server::accept_loop;
use crate::util::spawn_and_log_error;

mod eventbus;
mod http_flv;
mod protocol;
mod rtmp_server;
mod util;
mod ws_h264;
mod ws_fmp4;

#[derive(Clap, Debug)]
#[clap(version=crate_version!(), author = "Ninthakeey <ninthakeey@hotmail.com>")]
struct Opts {
    #[clap(long, default_value = "8080")]
    http_flv_port: u16,
    #[clap(long, default_value = "8081")]
    ws_h264_port: u16,
    #[clap(long, default_value = "8082")]
    ws_fmp4_port: u16,
    #[clap(long, default_value = "1935")]
    rtmp_port: u16,
}


fn main() -> anyhow::Result<()> {
    util::init_logger();

    let opts: Opts = Opts::parse();
    log::info!("{:?}", &opts);

    spawn_and_log_error(http_flv::run_server(format!("0.0.0.0:{}", opts.http_flv_port)));
    spawn_and_log_error(ws_h264::run_server(format!("0.0.0.0:{}", opts.ws_h264_port)));
    spawn_and_log_error(ws_fmp4::run_server(format!("0.0.0.0:{}", opts.ws_fmp4_port)));
    smol::block_on(accept_loop(&format!("0.0.0.0:{}", opts.rtmp_port)))
}
