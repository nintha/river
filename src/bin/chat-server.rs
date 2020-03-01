use async_std::task;
use async_std::net::{TcpListener, ToSocketAddrs};
use async_std::prelude::*;
use async_std::{
    io::BufReader,
    net::TcpStream,
};

use futures::{select, FutureExt};
use futures::channel::mpsc;
use futures::sink::SinkExt;
use std::sync::Arc;
use std::collections::hash_map::{Entry, HashMap};

type BoxResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = mpsc::UnboundedSender<T>;
type Receiver<T> = mpsc::UnboundedReceiver<T>;


#[derive(Debug)]
enum Void {}

#[derive(Debug)]
enum Event {
    NewPeer {
        name: String,
        stream: Arc<TcpStream>,
        shutdown: Receiver<Void>,
    },
    Message {
        from: String,
        to: Vec<String>,
        msg: String,
    },
}


fn spawn_and_log_error<F>(fut: F) -> task::JoinHandle<()>
    where
        F: Future<Output=BoxResult<()>> + Send + 'static,
{
    task::spawn(async move {
        if let Err(e) = fut.await {
            eprintln!("{}", e)
        }
    })
}

/// 建立事件处理循环
async fn broker_loop(events: Receiver<Event>) {
    let (disconnect_sender, mut disconnect_receiver) = // 1
        mpsc::unbounded::<(String, Receiver<String>)>();

    let mut peers: HashMap<String, Sender<String>> = HashMap::new();
    let mut events = events.fuse();

    loop {
        let event = select! {
            event = events.next().fuse() => match event {
                None => break,
                Some(event) => event,
            },
            disconnect = disconnect_receiver.next().fuse() => {
                let (name, ..) = disconnect.unwrap();
                assert!(peers.remove(&name).is_some());
                continue;
            }
        };

        match event {
            Event::Message { from, to, msg } => {  // 3
                for addr in to {
                    if let Some(peer) = peers.get_mut(&addr) {
                        let msg = format!("from {}: {}\n", from, msg);
                        peer.send(msg).await.unwrap()
                    } else {
                        println!("not found peer, to={}, from={}", addr, from);
                    }
                }
            }
            Event::NewPeer { name, stream, shutdown } => {
                match peers.entry(name.clone()) {
                    Entry::Occupied(..) => (),
                    Entry::Vacant(entry) => {
                        let (client_sender, mut client_receiver) = mpsc::unbounded();
                        entry.insert(client_sender);
                        let mut disconnect_sender = disconnect_sender.clone();
                        spawn_and_log_error(async move {
                            let res = connection_writer_loop(&mut client_receiver, stream, shutdown).await;
                            disconnect_sender.send((name, client_receiver)).await.unwrap();
                            res
                        });
                    }
                }
            }
        }
    }

    drop(peers);
    drop(disconnect_sender);
    while let Some(_) = disconnect_receiver.next().await {}
    println!("broker_loop end");
}

/// 建立发送消息循环
async fn connection_writer_loop(
    messages: &mut Receiver<String>,
    stream: Arc<TcpStream>,
    shutdown: Receiver<Void>,
) -> BoxResult<()> {
    let mut stream = &*stream;
    let mut messages = messages.fuse();
    let mut shutdown = shutdown.fuse();
    loop {
        select! {
            msg = messages.next().fuse() => match msg {
                Some(msg) => stream.write_all(msg.as_bytes()).await?,
                None => break,
            },
            void = shutdown.next().fuse() => match void {
                Some(void) => match void {}, // 3
                None => break,
            }
        }
    }
    Ok(())
}

/// TCP 连接处理
async fn accept_loop(addr: impl ToSocketAddrs + std::fmt::Display) -> BoxResult<()> {
    println!("Server is listening to {}", &addr);
    let listener = TcpListener::bind(addr).await?;

    let (broker_sender, broker_receiver) = mpsc::unbounded(); // 1
    let broker_handle = task::spawn(broker_loop(broker_receiver));

    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        println!("Accepting from: {}", stream.peer_addr()?);
        spawn_and_log_error(connection_loop(broker_sender.clone(), stream));
    }
    drop(broker_sender);
    broker_handle.await;
    println!("accept_loop end");
    Ok(())
}


async fn connection_loop(mut broker: Sender<Event>, stream: TcpStream) -> BoxResult<()> {
    let stream = Arc::new(stream);
    let reader = BufReader::new(&*stream); // 2
    let mut lines = reader.lines();

    let name = match lines.next().await { // 3
        None => Err("peer disconnected immediately")?,
        Some(line) => line?,
    };
    println!("name = {}", name);

    let (_shutdown_sender, shutdown_receiver) = mpsc::unbounded::<Void>();
    broker.send(Event::NewPeer {
        name: name.clone(),
        stream: Arc::clone(&stream),
        shutdown: shutdown_receiver,
    }).await.unwrap();


    while let Some(line) = lines.next().await { // 4
        let line = line?;
        let (dest, msg) = match line.find(':') { // 5
            None => continue,
            Some(idx) => (&line[..idx], line[idx + 1..].trim()),
        };
        let dest: Vec<String> = dest.split(',').map(|name| name.trim().to_string()).collect();
        let msg: String = msg.to_string();
        println!("dest={:?}, msg={}", dest, msg);

        broker.send(Event::Message {
            from: name.clone(),
            to: dest,
            msg,
        }).await.unwrap();
    }
    Err("peer disconnected immediately")?
}


fn main() -> BoxResult<()> {
    let server = "127.0.0.1:12345";
    task::block_on(accept_loop(server))
}
