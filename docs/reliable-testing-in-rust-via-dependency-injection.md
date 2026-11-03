# 🛡️ Reliable Testing in Rust via Dependency Injection

Writing robust, reliable, and parallelizable tests requires an intentional
approach to handling external dependencies such as environment variables, the
filesystem, or the system clock. Functions that directly call `std::env::var` or
`SystemTime::now()` are difficult to test because they depend on global,
non-deterministic state.

This leads to several problems:

- **Flaky Tests:** A test might pass or fail depending on the environment it
  runs in.
- **Parallel Execution Conflicts:** Tests that modify the same global
  environment variable (`std::env::set_var`) will interfere with each other
  when run with `cargo test`.
- **State Corruption:** A test that panics can fail to clean up its changes to
  the environment, poisoning subsequent tests.

The solution is a classic software design pattern: **Dependency Injection
(DI)**. Instead of a function reaching out to the global state, its
dependencies are provided as arguments. The
[mockable](https://docs.rs/mockable/latest/mockable/) crate offers a convenient
set of traits (`Env`, `Clock`, etc.) to implement this pattern for common
system interactions in Rust.

______________________________________________________________________

## ✨ Mocking Environment Variables

### 1. Add `mockable`

First, add the crate to `Cargo.toml`: production code needs the runtime
dependency, and tests add the `mock` feature on top of it.

```toml
[dependencies]
chrono = "0.4"
mockable = { version = "3.0.0", features = ["clock"] }

[dev-dependencies]
mockable = { version = "3.0.0", features = ["clock", "mock"] }
```

The runtime dependency supplies `DefaultEnv` and `DefaultClock`; the `clock`
feature adds the `Clock` trait, whose methods use `chrono` types, so `chrono`
must also be a direct dependency. The `mock` feature in `[dev-dependencies]`
adds `MockEnv` and `MockClock`.

### 2. The Untestable Code (Before)

Directly calling `std::env` makes it hard to test all logic paths.

```rust
pub fn get_api_key() -> Option<String> {
    match std::env::var("API_KEY") {
        Ok(key) if !key.is_empty() => Some(key),
        _ => None,
    }
}
```

### 3. Refactoring for Testability (After)

The function is refactored to accept a generic type that implements the
`mockable::Env` trait.

```rust
use mockable::Env;

pub fn get_api_key(env: &impl Env) -> Option<String> {
    match env.string("API_KEY") {
        Some(key) if !key.is_empty() => Some(key),
        _ => None,
    }
}
```

The function's core logic remains unchanged, but its dependency on the
environment is now explicit and injectable.

### 4. Writing Isolated Unit Tests

Tests can use `MockEnv`, an in-memory mock, to simulate any environmental
condition without touching the actual process environment. `MockEnv` is a
`mockall` mock, so each test configures expectations instead of writing values
into it.

The lookup calls `Env::string`, which returns `Option<String>`, so the tests
stub that method with `expect_string`.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use mockable::{Env, MockEnv};

    #[test]
    fn test_get_api_key_present() {
        let mut env = MockEnv::new();
        env.expect_string()
            .withf(|key| key == "API_KEY")
            .returning(|_| Some("secret123".to_string()));
        assert_eq!(get_api_key(&env), Some("secret123".to_string()));
    }

    #[test]
    fn test_get_api_key_missing() {
        let mut env = MockEnv::new();
        env.expect_string().returning(|_| None);
        assert_eq!(get_api_key(&env), None);
    }

    #[test]
    fn test_get_api_key_present_but_empty() {
        let mut env = MockEnv::new();
        env.expect_string().returning(|_| Some(String::new()));
        assert_eq!(get_api_key(&env), None);
    }
}
```

These tests are fast, completely isolated from each other, and will never fail
due to external state.

### 5. Usage in Production Code

In production code, inject the default implementation, `DefaultEnv`, which
reads the actual process environment.

```rust
use mockable::DefaultEnv;

fn main() {
    let env = DefaultEnv;
    if let Some(api_key) = get_api_key(&env) {
        println!("API Key found!");
    } else {
        println!("API Key not configured.");
    }
}
```

______________________________________________________________________

## 🔩 Handling Other Non-Deterministic Dependencies

This dependency injection pattern also applies to other non-deterministic
dependencies, such as the system clock. `mockable` provides a `Clock` trait for
this purpose.

### Untestable Code

```rust
use chrono::{DateTime, Utc};

fn is_cache_entry_stale(creation_time: DateTime<Utc>) -> bool {
    let timeout = chrono::Duration::seconds(300);
    Utc::now().signed_duration_since(creation_time) > timeout
}
```

### Testable Refactor

```rust
use chrono::{DateTime, Duration, Utc};
use mockable::Clock;

fn is_cache_entry_stale(creation_time: DateTime<Utc>, clock: &impl Clock) -> bool {
    let timeout = Duration::seconds(300);
    clock.utc().signed_duration_since(creation_time) > timeout
}
```

### Testing with `MockClock`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mockable::{Clock, MockClock};

    fn mock_clock(now: DateTime<Utc>) -> MockClock {
        let mut clock = MockClock::new();
        clock.expect_utc().returning(move || now);
        clock
    }

    #[test]
    fn test_cache_is_not_stale() {
        let creation_time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let clock = mock_clock(creation_time + Duration::seconds(100));
        assert!(!is_cache_entry_stale(creation_time, &clock));
    }

    #[test]
    fn test_cache_is_stale() {
        let creation_time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let clock = mock_clock(creation_time + Duration::seconds(301));
        assert!(is_cache_entry_stale(creation_time, &clock));
    }
}
```

In production, an instance of `DefaultClock` would be used.

______________________________________________________________________

## 📌 Key Takeaways

- **The Problem is Non-Determinism:** Directly accessing global state like
  `std::env` or `SystemTime::now` makes code hard to test.
- **The Solution is Dependency Injection:** Pass dependencies into functions as
  arguments.
- **Use** `mockable` **Traits:** Abstract dependencies behind traits such as
  `impl Env` or `impl Clock`.
- **`Mock*` for Tests:** Use `MockEnv` and `MockClock` in unit tests for
  isolated, deterministic control.
- **`Default*` for Production:** Use `DefaultEnv` and `DefaultClock` in the
  application to interact with the actual system.
- **`DefaultEnv` is NOT a Scope Guard:** `DefaultEnv` reads the global process
  environment without automatic cleanup. The workspace policy prohibits
  mutating the parent test process, so integration tests that need a controlled
  environment configure the child with `Command::env_clear` and explicit
  `Command::env` calls instead of a guard crate. For unit tests, `MockEnv` is
  preferable. See the environment seam rule in the
  [developers' guide](developers-guide.md#example-and-benchmark-support).
