# TODO for Local Testing

Since Rust/Cargo is not installed on this system, follow these steps to test the implementation:

## 1. Install Rust

```bash
# On Linux/macOS:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# On Windows:
# Download and run rustup-init.exe from https://rustup.rs/
```

## 2. Verify Installation

```bash
rustc --version
cargo --version
```

## 3. Build the Project

```bash
cd c:\Users\xf\Desktop\mcp\mcp-sentinel

# Check for compilation errors
cargo check

# Build release binary
cargo build --release

# The binary will be at: target/release/mcp-sentinel.exe (Windows)
```

## 4. Create Configuration

```bash
# Copy example config
cp sentinel.toml.example sentinel.toml

# Edit with your actual MCP server configurations
# Make sure to set environment variables like GITHUB_TOKEN
```

## 5. Run the Gateway

```bash
# Start in foreground
cargo run --release -- start

# Or run the binary directly
./target/release/mcp-sentinel start
```

## 6. Test the API

```bash
# Check health
curl http://localhost:3000/health

# Test tool search
curl -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

## Expected Issues to Fix

Since the code was not compiled, there may be:

1. **Import errors**: Missing `use` statements
2. **Type mismatches**: Incorrect type conversions
3. **Async/await issues**: Missing `.await` calls
4. **Lifetime errors**: Rust borrow checker complaints

Run `cargo check` first to see all compilation errors, then fix them one by one.

## Week 1 Completion Checklist

- [x] Project structure created
- [x] All source files written
- [x] Configuration system implemented
- [x] Backend manager (stdio + HTTP)
- [x] Health tracking (in-memory)
- [x] TF-IDF semantic router
- [x] Gateway server with 4 meta-tools
- [x] CLI interface
- [x] README documentation
- [ ] Compilation verification (requires Rust installation)
- [ ] Runtime testing with real MCP servers

## Next: Week 2 Tasks

Once Week 1 compiles and runs successfully:

1. Add SQLite storage (`rusqlite`, `tokio-rusqlite` deps)
2. Implement persistent health tracking
3. Add p95 latency calculation with sliding window
4. Implement `report` CLI command
5. Enhance zombie detection logic
6. Add daily aggregation background task
