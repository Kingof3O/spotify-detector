#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct ArtworkSnapshot {
    pub bytes: Arc<[u8]>,
    pub content_type: String,
    pub version: u64,
}

#[derive(Clone, Default)]
pub struct ArtworkStore {
    inner: Arc<RwLock<ArtworkState>>,
}

#[derive(Default)]
struct ArtworkState {
    current: Option<ArtworkSnapshot>,
    next_version: u64,
}

impl ArtworkStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn snapshot(&self) -> Option<ArtworkSnapshot> {
        self.inner.read().await.current.clone()
    }

    pub async fn replace(&self, bytes: Vec<u8>, content_type: String) -> u64 {
        let mut state = self.inner.write().await;

        if let Some(current) = &state.current {
            if current.bytes.as_ref() == bytes.as_slice() && current.content_type == content_type {
                return current.version;
            }
        }

        let version = next_version(&mut state.next_version);
        state.current = Some(ArtworkSnapshot {
            bytes: Arc::from(bytes.into_boxed_slice()),
            content_type,
            version,
        });
        version
    }

    pub async fn clear(&self) -> u64 {
        let mut state = self.inner.write().await;
        if state.current.is_some() {
            let version = next_version(&mut state.next_version);
            state.current = None;
            version
        } else {
            state.next_version
        }
    }
}

fn next_version(version: &mut u64) -> u64 {
    *version = version.wrapping_add(1).max(1);
    *version
}

#[cfg(test)]
mod tests {
    use super::ArtworkStore;

    #[tokio::test]
    async fn artwork_versions_change_only_when_content_changes() {
        let store = ArtworkStore::new();
        assert!(store.snapshot().await.is_none());

        let first = store.replace(vec![1, 2, 3], "image/png".to_owned()).await;
        let same = store.replace(vec![1, 2, 3], "image/png".to_owned()).await;
        let changed = store.replace(vec![4, 5, 6], "image/png".to_owned()).await;

        assert_eq!(first, same);
        assert!(changed > first);
        assert_eq!(
            store.snapshot().await.expect("artwork").bytes.as_ref(),
            &[4, 5, 6]
        );
    }

    #[tokio::test]
    async fn clearing_artwork_removes_the_current_snapshot() {
        let store = ArtworkStore::new();
        store.replace(vec![1], "image/jpeg".to_owned()).await;
        let version = store.clear().await;

        assert!(version > 0);
        assert!(store.snapshot().await.is_none());
    }
}
