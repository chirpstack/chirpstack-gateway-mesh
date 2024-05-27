use std::{collections::VecDeque, usize};

use crate::packets;

pub struct Cache<T> {
    deque: VecDeque<T>,
    size: usize,
}

impl<T> Cache<T> {
    pub fn new(size: usize) -> Cache<T> {
        Cache {
            deque: VecDeque::with_capacity(size),
            size,
        }
    }

    // Add a value to the cache. Returns true when the item was added, returns false when the item
    // already exists in the cache and was not added.
    pub fn add(&mut self, value: T) -> bool
    where
        T: PartialEq,
    {
        if self.deque.contains(&value) {
            return false;
        }

        if self.deque.len() == self.size {
            self.deque.pop_front();
        }
        self.deque.push_back(value);
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PayloadCache {
    p_type: packets::PayloadType,
    uplink_id: u16,
    relay_id: [u8; 4],
}

impl From<&packets::RelayPacket> for PayloadCache {
    fn from(p: &packets::RelayPacket) -> PayloadCache {
        let p_type = p.mhdr.payload_type;

        match &p.payload {
            packets::Payload::Uplink(v) => PayloadCache {
                p_type,
                uplink_id: v.metadata.uplink_id,
                relay_id: v.relay_id,
            },
            packets::Payload::Downlink(v) => PayloadCache {
                p_type,
                uplink_id: v.metadata.uplink_id,
                relay_id: v.relay_id,
            },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_cache() {
        let mut cache: Cache<usize> = Cache::new(5);
        assert!(cache.deque.is_empty());

        assert!(cache.add(1));
        assert!(!cache.add(1));
        assert!(cache.add(2));
        assert!(cache.add(3));
        assert!(cache.add(4));
        assert!(cache.add(5));

        assert_eq!(5, cache.deque.len());
        assert!(cache.add(6));
        assert_eq!(5, cache.deque.len());
    }
}
