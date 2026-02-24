//! Concurrent type-erased application data store for shared state.
//!
//! `AppDataStore` stores one value per concrete type, keyed by `TypeId`. Values
//! are stored in `Arc<dyn Any + Send + Sync>` to allow cheap cloning and safe
//! sharing across threads. The underlying `DashMap` provides lock-free
//! concurrent reads and sharded writes, enabling multiple threads to insert
//! and retrieve state simultaneously without external synchronisation.
//! Typed accessors provide a small API surface while hiding the underlying
//! type-erasure details.

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use dashmap::DashMap;

/// Stores application-scoped state values keyed by concrete type.
///
/// `AppDataStore` is used by `WireframeApp` and `MessageRequest` to share
/// connection-independent state with extractors without exposing the underlying
/// type-erasure map.
///
/// # Examples
///
/// ```rust
/// use wireframe::app_data_store::AppDataStore;
///
/// let store = AppDataStore::default();
/// store.insert(42u32);
/// let value = store.get::<u32>().expect("value should exist");
/// assert_eq!(*value, 42);
/// ```
#[derive(Clone, Default)]
pub struct AppDataStore {
    values: DashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl AppDataStore {
    /// Insert a value of type `T` into the store.
    ///
    /// Concurrent calls to `insert` from multiple threads are safe. Any
    /// existing value of the same type is replaced.
    ///
    /// # Parameters
    /// - `value`: The value to store. Any existing value of the same type is replaced.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::app_data_store::AppDataStore;
    ///
    /// let store = AppDataStore::default();
    /// store.insert("hello".to_string());
    /// ```
    pub fn insert<T>(&self, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.values.insert(
            TypeId::of::<T>(),
            Arc::new(value) as Arc<dyn Any + Send + Sync>,
        );
    }

    /// Retrieve a shared value of type `T`, if present.
    ///
    /// # Returns
    /// An `Arc<T>` when the value is present, or `None` if no value of type `T`
    /// has been registered.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::app_data_store::AppDataStore;
    ///
    /// let store = AppDataStore::default();
    /// store.insert(5u32);
    /// let value = store.get::<u32>().expect("value should be present");
    /// assert_eq!(*value, 5);
    /// ```
    #[must_use]
    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|guard| Arc::clone(guard.value()).downcast::<T>().ok())
    }

    /// Check whether a value of type `T` is present in the store.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::app_data_store::AppDataStore;
    ///
    /// let store = AppDataStore::default();
    /// assert!(!store.contains::<u32>());
    /// store.insert(42u32);
    /// assert!(store.contains::<u32>());
    /// ```
    #[must_use]
    pub fn contains<T>(&self) -> bool
    where
        T: 'static,
    {
        self.values.contains_key(&TypeId::of::<T>())
    }

    /// Remove a value of type `T` from the store, returning it if present.
    ///
    /// Concurrent calls to `remove` from multiple threads are safe.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use wireframe::app_data_store::AppDataStore;
    ///
    /// let store = AppDataStore::default();
    /// store.insert(42u32);
    /// let removed = store.remove::<u32>();
    /// assert_eq!(*removed.expect("value should have been present"), 42);
    /// assert!(!store.contains::<u32>());
    /// ```
    #[must_use]
    pub fn remove<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.values
            .remove(&TypeId::of::<T>())
            .and_then(|(_, arc)| arc.downcast::<T>().ok())
    }
}

#[cfg(test)]
#[expect(
    unused_braces,
    reason = "rstest fixture proc-macro consumes item-level attributes before clippy sees them"
)]
mod tests {
    //! Unit tests for [`AppDataStore`] covering insertion, retrieval,
    //! removal, containment checks, and concurrent access.

    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use rstest::{fixture, rstest};

    use super::AppDataStore;

    #[derive(Debug, PartialEq)]
    struct CustomState {
        label: &'static str,
        value: u32,
    }

    #[fixture]
    fn empty_store() -> AppDataStore { AppDataStore::default() }

    fn assert_send_sync<T: Send + Sync>() {}

    #[rstest]
    fn insert_and_get_multiple_types(empty_store: AppDataStore) {
        empty_store.insert(12u32);
        empty_store.insert("hello".to_string());
        empty_store.insert(CustomState {
            label: "alpha",
            value: 7,
        });

        let number = empty_store.get::<u32>().expect("u32 should be present");
        assert_eq!(*number, 12);

        let text = empty_store
            .get::<String>()
            .expect("String should be present");
        assert_eq!(text.as_str(), "hello");

        let custom = empty_store
            .get::<CustomState>()
            .expect("CustomState should be present");
        assert_eq!(
            *custom,
            CustomState {
                label: "alpha",
                value: 7,
            }
        );
    }

    #[rstest]
    fn insert_overwrites_existing_value(empty_store: AppDataStore) {
        empty_store.insert(10u32);
        empty_store.insert(20u32);

        let number = empty_store.get::<u32>().expect("u32 should be present");
        assert_eq!(*number, 20);
    }

    #[rstest]
    fn missing_type_returns_none(empty_store: AppDataStore) {
        assert!(empty_store.get::<u32>().is_none());
    }

    #[rstest]
    fn contains_returns_true_for_present_type(empty_store: AppDataStore) {
        assert!(!empty_store.contains::<u32>());
        empty_store.insert(42u32);
        assert!(empty_store.contains::<u32>());
    }

    #[rstest]
    fn remove_returns_and_deletes_value(empty_store: AppDataStore) {
        empty_store.insert(42u32);
        let removed = empty_store.remove::<u32>().expect("u32 should be present");
        assert_eq!(*removed, 42);
        assert!(empty_store.get::<u32>().is_none());
    }

    #[rstest]
    fn remove_returns_none_for_absent_type(empty_store: AppDataStore) {
        assert!(empty_store.remove::<u32>().is_none());
    }

    #[test]
    fn store_is_send_and_sync() { assert_send_sync::<AppDataStore>(); }

    #[rstest]
    fn concurrent_insert_and_get(empty_store: AppDataStore) {
        let store = Arc::new(empty_store);
        let barrier = Arc::new(Barrier::new(3));

        let handles: Vec<_> = vec![
            {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.insert(42u32);
                })
            },
            {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.insert("hello".to_string());
                })
            },
            {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.insert(CustomState {
                        label: "concurrent",
                        value: 99,
                    });
                })
            },
        ];

        for handle in handles {
            handle.join().expect("thread should not panic");
        }

        let number = store.get::<u32>().expect("u32 should be present");
        assert_eq!(*number, 42);
        let text = store.get::<String>().expect("String should be present");
        assert_eq!(text.as_str(), "hello");
        let custom = store
            .get::<CustomState>()
            .expect("CustomState should be present");
        assert_eq!(
            *custom,
            CustomState {
                label: "concurrent",
                value: 99,
            }
        );
    }

    #[rstest]
    fn concurrent_overwrite_converges(empty_store: AppDataStore) {
        let store = Arc::new(empty_store);
        let thread_count = 8;
        let barrier = Arc::new(Barrier::new(thread_count));

        let handles: Vec<_> = (0..thread_count)
            .map(|i| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "thread_count is well within u32 range"
                    )]
                    store.insert(i as u32);
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread should not panic");
        }

        // One of the threads' values must have "won".
        let value = store.get::<u32>().expect("u32 should be present");
        #[expect(
            clippy::cast_possible_truncation,
            reason = "thread_count is well within u32 range"
        )]
        let upper = thread_count as u32;
        assert!(*value < upper);
    }
}
