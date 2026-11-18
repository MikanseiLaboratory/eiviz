//! One-shot control-ws mutation probe: connects with the crate's own client,
//! sends a Mutation JSON file, prints the server's status. Dev tool only.
use eiviz_api::client::ControlClient;

#[tokio::main]
async fn main() {
    let mutation =
        std::fs::read(std::env::args().nth(1).expect("mutation json")).expect("read mutation");
    let client = ControlClient::websocket("ws://127.0.0.1:9400", "");
    match client.mutate_session(mutation, 0).await {
        Ok(()) => println!("mutate ok"),
        Err(error) => println!("mutate error: {error}"),
    }
}
