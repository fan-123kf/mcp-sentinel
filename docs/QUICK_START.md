# Quick Start Guide

Get mcp-sentinel running in 5 minutes.

## Prerequisites

- **Rust 1.75+** - Install from [rustup.rs](https://rustup.rs/)
- **Node.js 18+** - Install from [nodejs.org](https://nodejs.org/)
- **Git** - For cloning the repository

## Step 1: Clone and Build

```bash
# Clone the repository
git clone https://github.com/yourusername/mcp-sentinel.git
cd mcp-sentinel

# Build release version
cargo build --release
```

This will take 2-5 minutes on the first build.

## Step 2: Configure

### Option A: Minimal Config (No API Keys)

Perfect for testing:

```bash
# Use the minimal example
cp examples/minimal.toml sentinel.toml
```

This uses only the filesystem server, which works out of the box.

### Option B: Full Config (with GitHub)

```bash
# Copy example config
cp sentinel.toml.example sentinel.toml

# Set your GitHub token
export GITHUB_TOKEN="ghp_your_token_here"
```

Get a GitHub token at: https://github.com/settings/tokens

## Step 3: Start the Gateway

```bash
# Start the server
cargo run --release -- start
```

You should see:
```
INFO Starting mcp-sentinel gateway...
INFO Configuration loaded from sentinel.toml
INFO Gateway will listen on port 3000
INFO Loaded 6 tools from backends
🚀 mcp-sentinel gateway listening on http://0.0.0.0:3000
   MCP endpoint: http://0.0.0.0:3000/mcp
   Health check: http://0.0.0.0:3000/health
```

## Step 4: Verify It Works

Open a new terminal:

```bash
# Check health
curl http://localhost:3000/health

# Should return:
{
  "status": "healthy",
  "tools": {
    "total": 6,
    "healthy": 6,
    "degraded": 0,
    "zombie": 0
  }
}
```

## Step 5: Connect Your AI Client

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%/Claude/claude_desktop_config.json` (Windows):

```json
{
  "mcpServers": {
    "sentinel": {
      "url": "http://localhost:3000/mcp"
    }
  }
}
```

Restart Claude Desktop.

### Cursor IDE

Edit Cursor settings (`.cursor/config.json`):

```json
{
  "mcpServers": {
    "sentinel": {
      "url": "http://localhost:3000/mcp"
    }
  }
}
```

Restart Cursor.

## Step 6: Test in AI Client

In Claude or Cursor, try:

> "Search for tools that can list files"

The AI will now call `gateway_search_tools` and see only relevant tools!

---

## What's Next?

### Add More Backends

Edit `sentinel.toml`:

```toml
# Add Linear
[backends.linear]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-linear"]
env = { LINEAR_API_KEY = "${LINEAR_API_KEY}" }

# Add Slack
[backends.slack]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-slack"]
env = { SLACK_BOT_TOKEN = "${SLACK_BOT_TOKEN}" }
```

Then restart the gateway.

### View Health Reports

```bash
# Generate report
mcp-sentinel report

# Save to file
mcp-sentinel report --output health.md

# View for last 14 days
mcp-sentinel report --days 14
```

### List All Tools

```bash
# List tools sorted by health
mcp-sentinel tools list

# Sort by tool ID
mcp-sentinel tools list --sort-by tool_id
```

### Monitor Performance

```bash
# Watch logs in real-time
RUST_LOG=info mcp-sentinel start

# Debug mode (verbose)
RUST_LOG=debug mcp-sentinel start
```

---

## Troubleshooting

### Gateway won't start?

1. Check port 3000 is not in use:
```bash
lsof -i :3000  # macOS/Linux
netstat -ano | findstr :3000  # Windows
```

2. Check config file exists:
```bash
ls -la sentinel.toml
```

3. View detailed errors:
```bash
RUST_LOG=debug cargo run -- start
```

### No tools showing up?

1. Check backend initialization logs - you should see:
```
INFO Initializing backend: filesystem
INFO Backend filesystem loaded 6 tools
```

2. Test backend manually:
```bash
npx -y @modelcontextprotocol/server-filesystem .
```

3. Check health endpoint:
```bash
curl http://localhost:3000/health | jq
```

### Need more help?

See [TROUBLESHOOTING.md](./TROUBLESHOOTING.md) for detailed solutions.

---

## Example Workflows

### 1. Find and Use a Tool

**In AI client:**
```
User: "Create a GitHub issue in my repo"

AI: [calls gateway_search_tools("create github issue")]
    [receives: github::create_issue with 0.95 health score]
    [calls gateway_invoke with github::create_issue]
```

### 2. Check System Health

**In terminal:**
```bash
mcp-sentinel report
```

**Output:**
```markdown
# MCP Sentinel Health Report

## Summary
- Total tools: 23
- Healthy: 20 (87%)
- Degraded: 1 (4%)
- Zombie: 2 (9%)

## Recommendations
1. Remove `obsidian` server — 2 zombie tools
2. Investigate `linear::update_issue` — 12% health score
```

### 3. Optimize Token Usage

**In AI client:**
```
User: "Suggest which tools I can remove"

AI: [calls gateway_suggest_cleanup]
```

**Result:**
```json
{
  "zombie_servers": [{
    "server": "obsidian",
    "zombie_count": 2,
    "reason": "2 tools unused for 7+ days, wasting ~300 tokens/turn"
  }],
  "estimated_token_savings": 300
}
```

Remove from `sentinel.toml` and restart!

---

## Configuration Options

### Key Settings

| Option | Default | Description |
|--------|---------|-------------|
| `gateway.port` | 3000 | HTTP server port |
| `routing.top_k` | 5 | Max results from search |
| `routing.health_weight` | 0.4 | Health influence on ranking (0-1) |
| `health.zombie_threshold_days` | 7 | Days inactive before zombie |
| `health.consecutive_failure_limit` | 5 | Failures before degraded |
| `storage.retention_days` | 30 | How long to keep call history |

### Tuning Tips

**For faster searches:**
```toml
[routing]
top_k = 3  # Return fewer results
```

**For aggressive cleanup:**
```toml
[health]
zombie_threshold_days = 3  # Flag unused tools faster
```

**For minimal storage:**
```toml
[storage]
retention_days = 7  # Keep only recent data
```

---

## Next Steps

- [Architecture Overview](../README.md#技术架构)
- [Troubleshooting Guide](./TROUBLESHOOTING.md)
- [Contributing](../README.md#贡献)

Enjoy using mcp-sentinel! 🚀
