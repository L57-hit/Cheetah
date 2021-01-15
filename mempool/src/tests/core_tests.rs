use super::*;
use crate::common::{committee, keys, payload};
use std::fs;
use std::time::Duration;
use tokio::sync::mpsc::channel;
use tokio::sync::oneshot;
use tokio::time::sleep;

async fn core(
    store_path: &str,
) -> (
    Receiver<NetMessage>,
    Sender<CoreMessage>,
    Sender<ConsensusMessage>,
    Sender<Transaction>,
) {
    let (tx_network, rx_network) = channel(1);
    let (tx_core, rx_core) = channel(1);
    let (tx_consensus, rx_consensus) = channel(1);
    let (tx_client, rx_client) = channel(1);

    let (name, secret) = keys().pop().unwrap();
    let config = Config {
        name,
        committee: committee(),
        parameters: Parameters {
            queue_capacity: 1,
            max_payload_size: 1,
        },
    };
    let signature_service = SignatureService::new(secret);
    let _ = fs::remove_dir_all(store_path);
    let store = Store::new(store_path).unwrap();
    let mut core = Core::new(
        config,
        signature_service,
        store,
        tx_network,
        rx_core,
        rx_consensus,
        rx_client,
    );
    tokio::spawn(async move {
        core.run().await;
    });

    (rx_network, tx_core, tx_consensus, tx_client)
}

#[tokio::test]
async fn test_handle_transaction() {
    // Run the core.
    let path = ".store_test_handle_transaction";
    let (mut rx_network, _tx_core, _tx_consensus, tx_client) = core(path).await;

    // Ensure the core transmits the payload to the network.
    tx_client.send(vec![1_u8]).await.unwrap();
    tx_client.send(vec![1_u8]).await.unwrap();
    assert!(rx_network.recv().await.is_some());
}

#[tokio::test]
async fn test_handle_request() {
    // Run the core.
    let path = ".store_test_handle_request";
    let (mut rx_network, tx_core, _tx_consensus, _tx_client) = core(path).await;

    // Send a payload to the core.
    let message = CoreMessage::Payload(payload());
    tx_core.send(message).await.unwrap();
    sleep(Duration::from_millis(100)).await;

    // Send a sync request.
    let (name, _) = keys().pop().unwrap();
    let digest = payload().digest();
    let message = CoreMessage::SyncRequest(digest, name);
    tx_core.send(message).await.unwrap();

    // Ensure we transmit a reply.
    assert!(rx_network.recv().await.is_some());
}

#[tokio::test]
async fn test_get_payload() {
    // Run the core.
    let path = ".store_test_get_payload";
    let (_rx_network, _tx_core, tx_consensus, tx_client) = core(path).await;

    // Send enough transactions to generate a payload.
    tx_client.send(vec![1_u8]).await.unwrap();
    tx_client.send(vec![1_u8]).await.unwrap();

    // Get the next payload.
    let (sender, receiver) = oneshot::channel();
    let message = ConsensusMessage::Get(sender);
    tx_consensus.send(message).await.unwrap();
    let result = receiver.await.unwrap();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), payload().digest());
}

#[tokio::test]
async fn test_verify_existing_payload() {
    // Run the core.
    let path = ".store_test_verify_existing_payload";
    let (_rx_network, tx_core, tx_consensus, _tx_client) = core(path).await;

    // Send a payload to the core.
    let message = CoreMessage::Payload(payload());
    tx_core.send(message).await.unwrap();
    sleep(Duration::from_millis(100)).await;

    // Verify a payload.
    let (sender, receiver) = oneshot::channel();
    let message = ConsensusMessage::Verify(payload().digest(), sender);
    tx_consensus.send(message).await.unwrap();
    let result = receiver.await.unwrap();
    assert!(result.unwrap());
}

#[tokio::test]
async fn test_verify_missing_payload() {
    // Run the core.
    let path = ".store_test_verify_missing_payload";
    let (_rx_network, _tx_core, tx_consensus, _tx_client) = core(path).await;

    // Verify a payload.
    let (sender, receiver) = oneshot::channel();
    let message = ConsensusMessage::Verify(payload().digest(), sender);
    tx_consensus.send(message).await.unwrap();
    let result = receiver.await.unwrap();
    assert!(!result.unwrap());
}