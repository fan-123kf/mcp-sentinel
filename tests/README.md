# Tests for mcp-sentinel

This directory contains integration tests for the mcp-sentinel gateway.

## Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_tfidf_basic_search

# Run with logging
RUST_LOG=debug cargo test
```

## Test Structure

### Unit Tests
Located in `src/*/mod.rs` files alongside the code:
- `src/router/tfidf.rs` - TF-IDF algorithm tests
- `src/health/types.rs` - Health tracking logic tests
- `src/storage/sqlite.rs` - Database operations tests

### Integration Tests
Located in `tests/`:
- `integration_test.rs` - End-to-end system tests

## Writing New Tests

### Unit Test Example

Add to the bottom of the module file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_function() {
        assert_eq!(my_function(2), 4);
    }
}
```

### Integration Test Example

Create a new file in `tests/`:

```rust
use mcp_sentinel::*;

#[tokio::test]
async fn test_gateway_startup() {
    // Test code here
}
```

## Future Test Coverage

- [ ] Full TF-IDF search accuracy tests
- [ ] Health score calculation edge cases
- [ ] Database persistence and recovery
- [ ] Concurrent request handling
- [ ] Backend failure scenarios
- [ ] Memory leak detection
- [ ] Performance benchmarks
