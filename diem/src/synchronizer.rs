use crate::core::CoreMessage;
use crate::crypto::Digest;
use crate::crypto::Hash as _;
use crate::error::{DiemError, DiemResult};
use crate::messages::{Block, QC};
use crate::network::NetMessage;
use crate::store::Store;
use futures::future::FutureExt as _;
use futures::select;
use futures::stream::futures_unordered::FuturesUnordered;
use futures::stream::StreamExt as _;
use log::{debug, error};
use std::collections::HashSet;
use tokio::sync::mpsc::{channel, Sender};

pub struct Synchronizer {
    store: Store,
    inner_channel: Sender<Block>,
}

impl Synchronizer {
    pub async fn new(
        store: Store,
        network_channel: Sender<NetMessage>,
        core_channel: Sender<CoreMessage>,
    ) -> Self {
        let (tx, mut rx) = channel(100);
        let synchronizer = Self {
            store: store.clone(),
            inner_channel: tx,
        };
        tokio::spawn(async move {
            let mut waiting = FuturesUnordered::new();
            let mut pending = HashSet::new();
            loop {
                select! {
                    message = rx.next().fuse() => {
                        if let Some(block) = message {
                            if pending.insert(block.digest()) {
                                let previous = block.previous();
                                let fut = Self::waiter(store.clone(), previous, block);
                                waiting.push(fut);
                                let sync_request = NetMessage::SyncRequest(previous);
                                if let Err(e) = network_channel.send(sync_request).await {
                                    panic!("Failed to send Sync Request to network: {}", e);
                                }
                            }
                        }
                    }
                    result = waiting.select_next_some() => {
                        match result {
                            Ok(block) => {
                                let _ = pending.remove(&block.digest());
                                let message = CoreMessage::Block(block);
                                if let Err(e) = core_channel.send(message).await {
                                    panic!("Synchronizer failed to send message through core channel: {}", e);
                                }
                            },
                            Err(e) => error!("{}", e)
                        }
                    }
                }
            }
        });
        synchronizer
    }

    async fn waiter(mut store: Store, wait_on: Digest, deliver: Block) -> DiemResult<Block> {
        let _ = store.notify_read(wait_on.to_vec()).await?;
        Ok(deliver)
    }

    async fn get_previous_block(&mut self, block: &Block) -> DiemResult<Option<Block>> {
        if block.qc == QC::genesis() {
            return Ok(Some(Block::genesis()));
        }
        let previous = block.previous();
        match self.store.read(previous.to_vec()).await? {
            Some(bytes) => {
                bincode::deserialize(&bytes).map_err(|e| DiemError::StoreError(e.to_string()))
            }
            None => {
                debug!("Requesting sync for block {:?}", previous);
                if let Err(e) = self.inner_channel.send(block.clone()).await {
                    panic!("Failed to send request to synchronizer: {}", e);
                }
                Ok(None)
            }
        }
    }

    pub async fn get_ancestors(
        &mut self,
        block: &Block,
    ) -> DiemResult<Option<(Block, Block, Block)>> {
        let b2 = match self.get_previous_block(block).await? {
            Some(b) => b,
            None => return Ok(None),
        };
        let b1 = self
            .get_previous_block(&b2)
            .await?
            .expect("We should have all ancestors of delivered blocks");
        let b0 = self
            .get_previous_block(&b1)
            .await?
            .expect("We should have all ancestors of delivered blocks");
        Ok(Some((b0, b1, b2)))
    }
}
