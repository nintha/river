use crossbeam_utils::atomic::AtomicCell;
use dashmap::DashMap;
use smol::channel::{Receiver, Sender};

pub struct EventBus<E> {
    label: String,
    incr_val: AtomicCell<u64>,
    tx_map: DashMap<u64, Sender<E>>,
}

impl<E: 'static + Clone> EventBus<E> {
    pub fn with_label(label: String) -> Self {
        Self {
            label,
            incr_val: Default::default(),
            tx_map: Default::default(),
        }
    }

    pub async fn publish(&self, val: E) {
        let mut dropped_senders: Vec<u64> = vec![];

        let keys: Vec<u64> = self.tx_map.iter().map(|x| x.key().to_owned()).collect();

        for key in keys {
            if let Some(entry) = self.tx_map.get(&key) {
                if let Err(_) = entry.send(val.clone()).await {
                    dropped_senders.push(key);
                }
            }
        }
        for key in dropped_senders.iter() {
            self.tx_map.remove(key);
            log::info!("[EventBus][{}] remove receiver {}", self.label, key);
        }
    }

    pub fn register_receiver(&self) -> Receiver<E> {
        let (tx, rx) = smol::channel::unbounded();

        let key = self.incr_val.fetch_add(1);
        self.tx_map.insert(key, tx);

        log::info!("[EventBus][{}] add receiver {}", self.label, key);
        rx
    }
}
