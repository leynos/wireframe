//! Type-erased application data store for shared state.
//!
//! `AppDataStore` stores one value per concrete type, keyed by `TypeId`. Values
//! are stored in `Arc<dyn Any + Send + Sync>` to allow cheap cloning and safe
//! sharing across threads. Typed accessors provide a small API surface while
//! hiding the underlying type-erasure details.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

/// Stores application-scoped state values keyed by concrete type.
///
/// `AppDataStore` is used by `WireframeApp` and `MessageRequest` to share
/// connection-independent state with extractors without exposing the underlying
/// type-erasure map.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::AppDataStore;
///
/// let mut store = AppDataStore::default();
/// store.insert(42u32);
/// let value = store.get::<u32>().expect("value should exist");
/// assert_eq!(*value, 42);
/// ```
#[derive(Clone, Default)]
pub struct AppDataStore {
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl AppDataStore {
    /// Insert a value of type `T` into the store.
    ///
    /// # Parameters
    /// - `value`: The value to store. Any existing value of the same type is replaced.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wireframe::AppDataStore;
    ///
    /// let mut store = AppDataStore::default();
    /// store.insert("hello".to_string());
    /// ```
    pub fn insert<T>(&mut self, value: T)
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
    /// ```rust,no_run
    /// use wireframe::AppDataStore;
    ///
    /// let mut store = AppDataStore::default();
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
            .and_then(|data| Arc::clone(data).downcast::<T>().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::AppDataStore;

    #[derive(Debug, PartialEq)]
    struct CustomState {
        label: &'static str,
        value: u32,
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn insert_and_get_multiple_types() {
        let mut store = AppDataStore::default();
        store.insert(12u32);
        store.insert("hello".to_string());
        store.insert(CustomState {
            label: "alpha",
            value: 7,
        });

        let number = store.get::<u32>().expect("u32 should be present");
        assert_eq!(*number, 12);

        let text = store.get::<String>().expect("String should be present");
        assert_eq!(text.as_str(), "hello");

        let custom = store
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

    #[test]
    fn insert_overwrites_existing_value() {
        let mut store = AppDataStore::default();
        store.insert(10u32);
        store.insert(20u32);

        let number = store.get::<u32>().expect("u32 should be present");
        assert_eq!(*number, 20);
    }

    #[test]
    fn missing_type_returns_none() {
        let store = AppDataStore::default();
        assert!(store.get::<u32>().is_none());
    }

    #[test]
    fn store_is_send_and_sync() { assert_send_sync::<AppDataStore>(); }
}
