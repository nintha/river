use clap::crate_version;
use clap::Clap;
use river::{ws_h264, ws_fmp4, util, http_flv};
use river::rtmp_server::accept_loop;
use river::util::spawn_and_log_error;


#[derive(Clap, Debug)]
#[clap(version=crate_version!(), author = "Ninthakeey <ninthakeey@hotmail.com>")]
struct Opts {
    #[clap(long, default_value = "18000")]
    http_flv_port: u16,
    #[clap(long, default_value = "18001")]
    ws_h264_port: u16,
    #[clap(long, default_value = "18002")]
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
