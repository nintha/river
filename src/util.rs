use smol::net::TcpStream;
use smol::channel::Sender;
use smol::io::AsyncWriteExt;

/// 把tcp stream写流用channel包装一层
pub fn wrap_tcp_stream_as_channel_sender(mut stream: TcpStream) -> Sender<Vec<u8>> {
    let (tx, rx) = smol::channel::unbounded::<Vec<u8>>();
    smol::spawn(async move{
        while let Ok(bytes) = rx.recv().await{
            stream.write_all(&bytes).await.unwrap();
        }
    }).detach();
    tx
}