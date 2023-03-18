use dotenvy::dotenv;
use openai::{
    chat::{ChatCompletion, ChatCompletionMessage, ChatCompletionMessageRole},
    models::ModelID,
};

use futures_util::{SinkExt, StreamExt};
use log::*;
use std::{net::SocketAddr, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Error};
use tungstenite::{Message, Result};

async fn accept_connection(peer: SocketAddr, stream: TcpStream) {
    if let Err(e) = handle_connection(peer, stream).await {
        match e {
            Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8 => (),
            err => error!("Error processing connection: {}", err),
        }
    }
}

async fn handle_message(prev_messages: &mut Vec<ChatCompletionMessage>, new_msg: &Message) -> Result<String> {
    prev_messages.push(ChatCompletionMessage {
        role: ChatCompletionMessageRole::User,
        content: new_msg.to_string(),
        name: None,
    });

    let chat_completion = ChatCompletion::builder(ModelID::Gpt3_5Turbo, prev_messages.clone())
        .create()
        .await
        .unwrap()
        .unwrap();
    let returned_message = chat_completion.choices.first().unwrap().message.clone();

    let reply = format!("AI: {}", returned_message.content.trim()); // TODO: Send whole `returned_message` as JSON

    prev_messages.push(returned_message);    

    Ok(reply)
}

async fn handle_connection(peer: SocketAddr, stream: TcpStream) -> Result<()> {
    let ws_stream = accept_async(stream).await.expect("Failed to accept");
    info!("New WebSocket connection: {}", peer);
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let mut interval = tokio::time::interval(Duration::from_millis(1000));

    // Send initial prompt to AI and send response back to websocket
    let mut prev_messages = vec![ChatCompletionMessage {
        role: ChatCompletionMessageRole::System,
        content: include_str!("initial-prompt.md").to_string(),
        name: None,
    }];
    let chat_completion = ChatCompletion::builder(ModelID::Gpt3_5Turbo, prev_messages.clone())
        .create()
        .await
        .unwrap()
        .unwrap();
    let initial_response = chat_completion.choices.first().unwrap().message.clone();
    ws_sender.send(format!("AI: {}", initial_response.content.trim()).into()).await?;
    prev_messages.push(initial_response);

    loop {
        tokio::select! {
            new_msg = ws_receiver.next() => {
                match new_msg {
                    Some(new_msg) => {
                        let new_msg = new_msg?;
                        if new_msg.is_text() || new_msg.is_binary() {
                            let reply = handle_message(&mut prev_messages, &new_msg).await?;
                            ws_sender.send(reply.into()).await?; // TODO: Should I reply here on inside `handle_message`?
                        } else if new_msg.is_close() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = interval.tick() => {
                ws_sender.send(Message::Text("Tick".to_owned())).await?;
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() {

    // Make sure you have a file named `.env` with the `OPENAI_KEY` environment variable defined!
    dotenv().expect(".env file not found");

    env_logger::init();

    let addr = "127.0.0.1:9002";
    let listener = TcpListener::bind(&addr).await.expect("Can't listen");
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let peer = stream.peer_addr().expect("connected streams should have a peer address");
        info!("Peer address: {}", peer);

        tokio::spawn(accept_connection(peer, stream));
    }
}
