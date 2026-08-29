# Example Configurations

This directory contains example configurations for different use cases.

## Available Examples

### 1. `minimal.toml` - Minimal Setup

Perfect for testing and development. Uses only the filesystem server, which requires no API keys.

**Use case**: 
- First-time users
- Testing the gateway
- Local development

**Start with**:
```bash
cp examples/minimal.toml sentinel.toml
cargo run --release -- start
```

### 2. Full Configuration

See `../sentinel.toml.example` for a complete configuration with multiple backends:
- GitHub (requires `GITHUB_TOKEN`)
- Filesystem (no token needed)
- Linear (HTTP backend example)

## Creating Your Own

Start with `minimal.toml` and add backends as needed:

```toml
# Add GitHub
[backends.github]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }

# Add Slack
[backends.slack]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-slack"]
env = { SLACK_BOT_TOKEN = "${SLACK_BOT_TOKEN}" }

# Add custom HTTP backend
[backends.custom]
transport = "http"
url = "http://localhost:4000/mcp"
auth = { type = "bearer", token = "${CUSTOM_TOKEN}" }
```

## Testing Configurations

Validate your config before starting:

```bash
# Test configuration loading
cargo run -- --config your-config.toml tools list

# Check which tools are available
cargo run -- --config your-config.toml report
```

## Common Patterns

### Development (Fast Iteration)
```toml
[storage]
db_path = "./dev-sentinel.db"  # Local database
retention_days = 1  # Minimal retention

[health]
zombie_threshold_days = 1  # Quick cleanup
```

### Production (Stable)
```toml
[storage]
db_path = "~/.config/mcp-sentinel/sentinel.db"
retention_days = 90  # Long retention

[health]
zombie_threshold_days = 14  # Conservative cleanup
consecutive_failure_limit = 10  # More tolerant
```

### Performance (Low Latency)
```toml
[routing]
top_k = 3  # Return fewer results
health_weight = 0.6  # Prioritize healthy tools

[storage]
retention_days = 7  # Smaller database
```
