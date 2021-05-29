use clap::crate_version;
use clap::Clap;
use river::{ws_h264, ws_fmp4, util, http_flv, http_player};
use river::rtmp_server::accept_loop;
use river::util::spawn_and_log_error;


#[derive(Clap, Debug)]
#[clap(version = crate_version ! (), author = "Ninthakeey <ninthakeey@hotmail.com>")]
struct Opts {
    #[clap(long, default_value = "0", about = "disabled if port is 0")]
    http_flv_port: u16,
    #[clap(long, default_value = "18000", about = "disabled if port is 0")]
    http_player_port: u16,
    #[clap(long, default_value = "18001", about = "disabled if port is 0")]
    ws_h264_port: u16,
    #[clap(long, default_value = "0", about = "disabled if port is 0")]
    ws_fmp4_port: u16,
    #[clap(long, default_value = "1935")]
    rtmp_port: u16,
}


fn main() -> anyhow::Result<()> {
    util::init_logger();

    let opts: Opts = Opts::parse();
    log::info!("{:?}", &opts);

    let player_html = include_str!("../static/player.html");
    let player_html = player_html.replace("{/*$INJECTED_CONTEXT*/}", &format!("{{port: {}}}", opts.ws_h264_port));

    if opts.http_player_port > 0 {
        spawn_and_log_error(http_player::run_server(format!("0.0.0.0:{}", opts.http_player_port), player_html));
    }
    if opts.http_flv_port > 0 {
        spawn_and_log_error(http_flv::run_server(format!("0.0.0.0:{}", opts.http_flv_port)));
    }
    if opts.ws_h264_port > 0 {
        spawn_and_log_error(ws_h264::run_server(format!("0.0.0.0:{}", opts.ws_h264_port)));
    }
    if opts.ws_fmp4_port > 0 {
        spawn_and_log_error(ws_fmp4::run_server(format!("0.0.0.0:{}", opts.ws_fmp4_port)));
    }
    smol::block_on(accept_loop(&format!("0.0.0.0:{}", opts.rtmp_port)))
}
