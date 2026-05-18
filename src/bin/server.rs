use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{channel, Sender};
use tokio_websockets::{Message, ServerBuilder, WebSocketStream};
use serde_json::json;

type Users = Arc<Mutex<Vec<String>>>;

async fn handle_connection(
    addr: SocketAddr,
    mut ws_stream: WebSocketStream<TcpStream>,
    bcast_tx: Sender<String>,
    users: Users,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut bcast_rx = bcast_tx.subscribe();
    let mut username: Option<String> = None;

    loop {
        tokio::select! {
            incoming = ws_stream.next() => {
                match incoming {
                    Some(Ok(msg)) => {
                        if let Some(text) = msg.as_text() {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
                                match parsed["messageType"].as_str().unwrap_or("") {
                                    "register" => {
                                        if let Some(name) = parsed["data"].as_str() {
                                            username = Some(name.to_string());

                                            // tambah ke users list
                                            {
                                                let mut u = users.lock().unwrap();
                                                if !u.contains(&name.to_string()) {
                                                    u.push(name.to_string());
                                                }
                                            }

                                            // broadcast users terbaru ke semua
                                            let user_list = users.lock().unwrap().clone();
                                            let response = json!({
                                                "messageType": "users",
                                                "dataArray": user_list
                                            }).to_string();
                                            let _ = bcast_tx.send(format!("__USERS__:{}", response));
                                        }
                                    }
                                    "message" => {
                                        if let Some(text_msg) = parsed["data"].as_str() {
                                            let from = username.clone().unwrap_or(addr.to_string());
                                            let inner = json!({
                                                "from": from,
                                                "message": text_msg,
                                                "time": 0
                                            }).to_string();
                                            let response = json!({
                                                "messageType": "message",
                                                "data": inner
                                            }).to_string();
                                            let _ = bcast_tx.send(format!("__MSG__:{}", response));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Some(Err(err)) => return Err(err.into()),
                    None => {
                        // client disconnect
                        if let Some(name) = &username {
                            let mut u = users.lock().unwrap();
                            u.retain(|x| x != name);
                            let user_list = u.clone();
                            drop(u);
                            let response = json!({
                                "messageType": "users",
                                "dataArray": user_list
                            }).to_string();
                            let _ = bcast_tx.send(format!("__USERS__:{}", response));
                        }
                        return Ok(());
                    }
                }
            }
            msg = bcast_rx.recv() => {
                let msg = msg?;
                if let Some(payload) = msg.strip_prefix("__MSG__:") {
                    ws_stream.send(Message::text(payload.to_string())).await?;
                } else if let Some(payload) = msg.strip_prefix("__USERS__:") {
                    ws_stream.send(Message::text(payload.to_string())).await?;
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (bcast_tx, _) = channel::<String>(16);
    let users: Users = Arc::new(Mutex::new(vec![]));

    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    println!("Listening on port 8080");

    loop {
        let (socket, addr) = listener.accept().await?;
        println!("New connection from {addr:?}");
        let bcast_tx2 = bcast_tx.clone();
        let users2 = users.clone();
        tokio::spawn(async move {
            let (_req, ws_stream) = ServerBuilder::new().accept(socket).await?;
            handle_connection(addr, ws_stream, bcast_tx2, users2).await
        });
    }
}