use smol::channel::{Sender, Receiver};
use dashmap::DashMap;
use crossbeam_utils::atomic::AtomicCell;

pub struct EventBus<E> {
    label: String,
    incr_val: AtomicCell<u64>,
    tx_map: DashMap<u64, Sender<E>>,
}

impl<E: 'static + Clone> EventBus<E> {
    pub fn with_label(label: impl ToString) -> Self {
        Self {
            label: label.to_string(),
            incr_val: Default::default(),
            tx_map: Default::default(),
        }
    }

    pub async fn publish(&self, val: E) {
        let mut dropped_senders: Vec<u64> = vec![];
        for el in self.tx_map.iter() {
            if let Err(_) = el.send(val.clone()).await {
                dropped_senders.push(*el.key());
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
