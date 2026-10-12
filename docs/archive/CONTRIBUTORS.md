# Contributing to mcp-sentinel

Thank you for considering contributing to mcp-sentinel! This document provides guidelines and instructions for contributing.

## 🎯 Ways to Contribute

### 1. Report Bugs

Found a bug? Please open an issue with:
- **Clear title**: "Bug: <short description>"
- **Environment**: OS, Rust version, Node.js version
- **Steps to reproduce**: Exact commands you ran
- **Expected vs actual behavior**: What should happen vs what happened
- **Logs**: Output with `RUST_LOG=debug`

### 2. Suggest Features

Have an idea? Open an issue with:
- **Clear title**: "Feature: <short description>"
- **Problem**: What problem does this solve?
- **Proposed solution**: How would it work?
- **Alternatives**: Other approaches you considered

### 3. Improve Documentation

- Fix typos or unclear sections
- Add examples or diagrams
- Translate to other languages
- Write blog posts or tutorials

### 4. Write Code

See [Development Setup](#development-setup) below.

## 🛠️ Development Setup

### Prerequisites

- Rust 1.75+ (from [rustup.rs](https://rustup.rs/))
- Node.js 18+ (for testing backends)
- Git

### Setup

```bash
# Fork and clone
git clone https://github.com/YOUR_USERNAME/mcp-sentinel.git
cd mcp-sentinel

# Create a branch
git checkout -b feature/your-feature-name

# Build and test
cargo build
cargo test
```

### Project Structure

```
src/
├── main.rs              # CLI entry point
├── config.rs            # Configuration parsing
├── backend/             # Backend connectors
│   ├── mod.rs
│   ├── stdio.rs
│   ├── http.rs
│   └── types.rs
├── health/              # Health tracking
│   ├── mod.rs
│   ├── tracker.rs
│   ├── diagnostics.rs
│   └── types.rs
├── router/              # Semantic routing
│   ├── mod.rs
│   ├── tfidf.rs
│   └── types.rs
├── storage/             # Database layer
│   ├── mod.rs
│   └── sqlite.rs
└── gateway/             # HTTP server
    ├── mod.rs
    └── meta_tools.rs
```

### Running Tests

```bash
# All tests
cargo test

# Specific test
cargo test test_tfidf_basic_search

# With output
cargo test -- --nocapture

# Integration tests only
cargo test --test integration_test
```

### Code Style

We use standard Rust formatting:

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt -- --check

# Lint
cargo clippy
cargo clippy -- -D warnings  # Fail on warnings
```

## 📝 Pull Request Process

### 1. Before Coding

- Check existing issues and PRs to avoid duplicates
- Open an issue to discuss large changes first
- Get feedback on approach before investing time

### 2. During Development

- Write tests for new functionality
- Update documentation (README, QUICK_START, etc.)
- Follow existing code style and patterns
- Add comments for complex logic
- Keep commits focused and atomic

### 3. Commit Messages

Use clear, descriptive commit messages:

```
feat: Add fallback routing for degraded tools

- Implement automatic fallback to alternative tools
- Add configuration option for fallback behavior
- Update documentation with fallback examples

Fixes #42
```

**Format**:
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `test:` - Adding or updating tests
- `refactor:` - Code refactoring
- `perf:` - Performance improvements
- `chore:` - Maintenance tasks

### 4. Pull Request

When ready, create a PR with:

**Title**: Clear description (e.g., "Add WebSocket transport support")

**Description**:
```markdown
## Summary
Brief description of what this PR does

## Changes
- List of specific changes
- Breaking changes (if any)

## Testing
How to test this change

## Related Issues
Fixes #123
Relates to #456
```

**Checklist**:
- [ ] Tests pass (`cargo test`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] No clippy warnings (`cargo clippy`)
- [ ] Documentation updated
- [ ] Examples updated (if needed)
- [ ] CHANGELOG.md updated (for notable changes)

### 5. Review Process

- Maintainers will review within 1-3 days
- Address feedback promptly
- Discussion and iteration are welcome
- Once approved, a maintainer will merge

## 🎨 Design Guidelines

### Code Principles

1. **Simplicity**: Prefer straightforward solutions
2. **Performance**: Keep search under 20ms
3. **Safety**: Leverage Rust's type system
4. **Modularity**: Clear separation of concerns
5. **Testing**: Cover edge cases

### API Design

- **Consistency**: Follow existing patterns
- **Extensibility**: Easy to add new features
- **Backward compatibility**: Avoid breaking changes
- **Clear errors**: Helpful error messages

### Documentation

- **Examples**: Show, don't just tell
- **Context**: Explain the "why", not just the "what"
- **Completeness**: Cover common use cases
- **Clarity**: Use simple language

## 🐛 Bug Triage

### Priority Levels

- **P0 - Critical**: Crashes, data loss, security
- **P1 - High**: Major functionality broken
- **P2 - Medium**: Important but has workaround
- **P3 - Low**: Minor issues, nice-to-have

### Labels

- `bug` - Something isn't working
- `enhancement` - New feature request
- `documentation` - Docs improvements
- `good first issue` - Good for newcomers
- `help wanted` - Extra attention needed
- `question` - Further information requested

## 💡 Feature Requests

### Evaluation Criteria

We consider:
1. **Alignment**: Fits project goals?
2. **Value**: Solves real problems?
3. **Complexity**: Implementation cost?
4. **Maintenance**: Long-term support burden?

### Roadmap

See [PROJECT_STATUS.md](PROJECT_STATUS.md) for planned features.

## 🧪 Testing Guidelines

### What to Test

- **Unit tests**: Pure logic, algorithms
- **Integration tests**: Component interactions
- **Edge cases**: Boundary conditions
- **Error paths**: Failure scenarios

### Test Structure

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_happy_path() {
        // Arrange
        let input = setup_test_data();
        
        // Act
        let result = function_under_test(input);
        
        // Assert
        assert_eq!(result, expected);
    }

    #[test]
    fn test_feature_edge_case() {
        // Test edge cases
    }

    #[test]
    #[should_panic(expected = "error message")]
    fn test_feature_error() {
        // Test error handling
    }
}
```

## 📚 Documentation Guidelines

### README.md

- Keep overview concise
- Link to detailed guides
- Update examples when adding features

### Code Comments

- Explain "why", not "what"
- Document public APIs
- Use `///` for doc comments
- Include examples in doc comments

```rust
/// Search for tools matching a natural language query.
///
/// # Arguments
///
/// * `query` - Natural language description
/// * `top_k` - Maximum number of results
///
/// # Returns
///
/// Vector of ranked tools sorted by relevance and health
///
/// # Example
///
/// ```
/// let results = router.search("create github issue", 5).await;
/// ```
pub async fn search(&self, query: &str, top_k: usize) -> Vec<RankedTool> {
    // Implementation
}
```

## 🌟 Good First Issues

Looking to contribute but don't know where to start?

Check issues labeled `good first issue`:
- Usually self-contained
- Clear acceptance criteria
- Mentorship available

## 🤝 Community Guidelines

- **Be respectful**: Treat everyone with kindness
- **Be constructive**: Focus on solutions
- **Be patient**: Maintainers are volunteers
- **Be collaborative**: Work together
- **Have fun**: Enjoy the process!

## 📄 License

By contributing, you agree that your contributions will be licensed under the MIT License.

## ❓ Questions?

- Open an issue with `question` label
- Tag `@maintainers` for urgent matters
- Check existing issues/discussions first

---

Thank you for contributing to mcp-sentinel! 🚀
