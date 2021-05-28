use smol::io::{AsyncWriteExt, AsyncReadExt};
use smol::net::{TcpListener, TcpStream};
use smol::stream::StreamExt;

use crate::util::spawn_and_log_error;

pub async fn run_server(addr: String, player_html: String) -> anyhow::Result<()> {
    // Open up a TCP connection and create a URL.
    let listener = TcpListener::bind(addr).await?;
    let addr = format!("http://{}", listener.local_addr()?);
    log::info!("HTTP-Player Server is listening to {}", addr);

    // For each incoming TCP connection, spawn a task and call `accept`.
    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        spawn_and_log_error(accept(stream, player_html.clone()));
    }
    Ok(())
}

async fn accept(mut stream: TcpStream, player_html: String) -> anyhow::Result<()> {
    log::info!("[HTTP] new connection from {}", stream.peer_addr()?);

    let mut buffer = [0; 1024];
    stream.read(&mut buffer).await?;

    let header = format!("HTTP/1.1 200 OK\r\n\
    Content-Type: text/html;charset=UTF-8\r\n\
    Connection: close\r\n\
    Content-Length: {}\r\n\
    Cache-Control: no-cache\r\n\
    Access-Control-Allow-Origin: *\r\n\
    \r\n\
    {}", player_html.len(), player_html);
    stream.write_all(header.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}
