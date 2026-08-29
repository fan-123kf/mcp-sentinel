# Troubleshooting Guide

This guide helps you resolve common issues when building and running mcp-sentinel.

## Table of Contents

- [Build Issues](#build-issues)
- [Runtime Issues](#runtime-issues)
- [Backend Connection Issues](#backend-connection-issues)
- [Performance Issues](#performance-issues)
- [Database Issues](#database-issues)

---

## Build Issues

### ❌ Compilation fails with missing dependencies

**Symptom**: 
```
error: failed to resolve patches for `https://github.com/rust-lang/crates.io-index`
```

**Solution**:
```bash
# Update Cargo.lock
cargo update

# Clean and rebuild
cargo clean
cargo build --release
```

### ❌ Linking errors on Windows

**Symptom**:
```
error: linking with `link.exe` failed
```

**Solution**:
1. Install Visual Studio Build Tools: https://visualstudio.microsoft.com/downloads/
2. Ensure "Desktop development with C++" workload is installed
3. Restart terminal and rebuild

### ❌ SQLite compilation fails

**Symptom**:
```
error: failed to compile `rusqlite`
```

**Solution**:
The project uses `bundled` feature for rusqlite, which should work out of the box. If it still fails:

```bash
# On Ubuntu/Debian
sudo apt-get install libsqlite3-dev

# On macOS
brew install sqlite3

# On Windows - usually works with bundled feature
```

---

## Runtime Issues

### ❌ "Config file not found"

**Symptom**:
```
Error: Failed to read config file: sentinel.toml
```

**Solution**:
```bash
# Copy the example config
cp sentinel.toml.example sentinel.toml

# Or specify a custom path
mcp-sentinel --config /path/to/config.toml start
```

### ❌ "Permission denied" for database

**Symptom**:
```
Error: Failed to create database directory: Permission denied
```

**Solution**:
```bash
# Create the directory manually
mkdir -p ~/.config/mcp-sentinel

# Or use a custom path in sentinel.toml
[storage]
db_path = "./sentinel.db"  # Use current directory
```

### ❌ Port already in use

**Symptom**:
```
Error: Address already in use (os error 98)
```

**Solution**:
```toml
# Change port in sentinel.toml
[gateway]
port = 3001  # Or any available port
```

Or kill the process using port 3000:
```bash
# Linux/macOS
lsof -ti:3000 | xargs kill -9

# Windows
netstat -ano | findstr :3000
taskkill /PID <PID> /F
```

---

## Backend Connection Issues

### ❌ "Failed to spawn process" for stdio backend

**Symptom**:
```
WARN Failed to initialize stdio backend github: Failed to spawn process
```

**Solution**:

1. **Check if Node.js is installed**:
```bash
node --version
npx --version
```

2. **Test the backend command manually**:
```bash
npx -y @modelcontextprotocol/server-github
```

3. **Check environment variables**:
```toml
# Ensure env vars are set
[backends.github]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }
```

```bash
# Set the token
export GITHUB_TOKEN="ghp_your_token_here"
```

### ❌ HTTP backend returns 401/403

**Symptom**:
```
Error: MCP error: Unauthorized
```

**Solution**:
```toml
# Verify auth configuration
[backends.linear]
transport = "http"
url = "http://localhost:4000/mcp"
auth = { type = "bearer", token = "${LINEAR_TOKEN}" }
```

```bash
# Set the token
export LINEAR_TOKEN="your_api_key"
```

### ❌ Backend tools not showing up

**Symptom**:
Gateway starts but `gateway_search_tools` returns empty results.

**Solution**:

1. **Check logs** for backend initialization:
```bash
RUST_LOG=debug mcp-sentinel start
```

2. **Verify backend is working**:
```bash
# Test manually
curl http://localhost:3000/health
```

3. **Check if backends are configured**:
```bash
# Look for lines like:
# INFO Initializing backend: github
# INFO Backend github loaded 15 tools
```

---

## Performance Issues

### ❌ High memory usage

**Symptom**:
Memory usage grows over time, especially with many backends.

**Solution**:

1. **Reduce retention period**:
```toml
[storage]
retention_days = 7  # Instead of 30
```

2. **Limit backend count**:
Remove unused backends from `sentinel.toml`

3. **Monitor with**:
```bash
# Check memory
ps aux | grep mcp-sentinel

# Or use htop
htop -p $(pgrep mcp-sentinel)
```

### ❌ Slow search responses

**Symptom**:
`gateway_search_tools` takes >1 second to respond.

**Solution**:

1. **Reduce top_k**:
```toml
[routing]
top_k = 3  # Instead of 5 or 10
```

2. **Check database size**:
```bash
ls -lh ~/.config/mcp-sentinel/sentinel.db
```

3. **Rebuild index** (restart gateway):
```bash
# Stop and restart
killall mcp-sentinel
mcp-sentinel start
```

---

## Database Issues

### ❌ "Database is locked"

**Symptom**:
```
Error: Database is locked
```

**Solution**:

1. **Check for multiple instances**:
```bash
ps aux | grep mcp-sentinel
# Kill duplicates
```

2. **Remove lock files**:
```bash
rm ~/.config/mcp-sentinel/sentinel.db-shm
rm ~/.config/mcp-sentinel/sentinel.db-wal
```

3. **Restart gateway**

### ❌ Corrupted database

**Symptom**:
```
Error: database disk image is malformed
```

**Solution**:

**⚠️ This will delete all historical data!**

```bash
# Backup first
cp ~/.config/mcp-sentinel/sentinel.db ~/sentinel-backup.db

# Delete and recreate
rm ~/.config/mcp-sentinel/sentinel.db*
mcp-sentinel start  # Will recreate schema
```

### ❌ "Disk full" when writing to database

**Symptom**:
```
Error: Failed to record tool call: disk full
```

**Solution**:

1. **Free up disk space**

2. **Reduce retention**:
```toml
[storage]
retention_days = 3  # Minimal retention
```

3. **Change database location**:
```toml
[storage]
db_path = "/mnt/external/sentinel.db"
```

---

## Getting More Help

### Enable debug logging

```bash
RUST_LOG=debug mcp-sentinel start 2>&1 | tee debug.log
```

### Check health endpoint

```bash
curl http://localhost:3000/health | jq
```

### Generate health report

```bash
mcp-sentinel report
```

### Report Issues

If you're still stuck:

1. **Collect information**:
   - OS and version: `uname -a` (Linux/macOS) or `systeminfo` (Windows)
   - Rust version: `rustc --version`
   - Node version: `node --version`
   - Error logs (with RUST_LOG=debug)

2. **Open an issue**: https://github.com/yourusername/mcp-sentinel/issues

3. **Include**:
   - What you were trying to do
   - What happened (error messages)
   - What you expected
   - Your configuration (redact sensitive tokens!)

---

## Quick Fixes Checklist

Before asking for help, try these:

- [ ] Restart the gateway
- [ ] Check config file exists (`sentinel.toml`)
- [ ] Verify environment variables are set
- [ ] Test backend commands manually
- [ ] Check logs with `RUST_LOG=debug`
- [ ] Verify port is not in use
- [ ] Ensure Node.js and Rust are installed
- [ ] Try the minimal example config (`examples/minimal.toml`)
