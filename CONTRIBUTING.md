# Contributing to ipv6ddns

Thank you for your interest in contributing to ipv6ddns! This document provides guidelines and instructions for contributing to the project.

## Development Setup

### Prerequisites

- Rust (latest stable, same as CI)
- Linux with netlink support (for development and testing)
- Cloudflare API Token with DNS edit permissions (for testing)
- Git

### Building the Project

```bash
# Clone the repository
git clone https://github.com/Neycrol/ipv6ddns.git
cd ipv6ddns

# Build the project in debug mode
cargo build

# Build the project in release mode
cargo build --release
```

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run tests for a specific module
cargo test cloudflare
cargo test netlink
```

### Running the Daemon

```bash
# Set environment variables
export CLOUDFLARE_API_TOKEN="your-token-here"
export CLOUDFLARE_ZONE_ID="your-zone-id"
export CLOUDFLARE_RECORD_NAME="home.example.com"

# Run the daemon
cargo run --release
```

### Building for Android

```bash
# Install Android SDK and NDK
# Set ANDROID_NDK_HOME environment variable

# Build Android assets
./scripts/ci/build-android.sh

# Build APK
cd android
gradle assembleRelease
```

## Coding Standards

### Rust Code Style

- Follow the official Rust style guide
- Use `cargo fmt` to format code
- Use `cargo clippy` to lint code
- Add rustdoc comments to all public APIs
- Use `anyhow` for error handling
- Use `tracing` for logging

### Code Organization

- Keep functions focused and small
- Use meaningful variable and function names
- Add comments for complex logic
- Prefer composition over inheritance
- Use traits for shared behavior

### Documentation

- Add rustdoc comments to all public APIs
- Include examples in documentation
- Keep README.md up to date
- Document any breaking changes

## Pull Request Guidelines

### Before Submitting a PR

1. **Check existing issues**: Look for existing issues or PRs related to your change
2. **Create a branch**: Create a new branch from `main` for your changes
3. **Write tests**: Add tests for new functionality
4. **Update documentation**: Update README.md and rustdoc as needed
5. **Run checks**: Ensure all tests pass and code is formatted

### PR Checklist

- [ ] Code follows the project's coding standards
- [ ] Tests are included for new functionality
- [ ] Documentation is updated
- [ ] `cargo fmt` has been run
- [ ] `cargo clippy` has been run with no warnings
- [ ] `cargo test` passes
- [ ] Commit messages are clear and descriptive
- [ ] PR description explains the change and motivation

### Commit Message Format

Use clear and descriptive commit messages:

```
feat: add feature description
fix: fix bug description
docs: update documentation
ci: update CI configuration
refactor: refactor code without changing behavior
perf: improve performance
test: add or update tests
```

Examples:
- `feat: add IPv6 address validation`
- `fix: handle rate limiting from Cloudflare API`
- `docs: add CONTRIBUTING.md`
- `ci: add security scanning workflow`

### PR Description

Include a clear description of your changes:

- **Summary**: Brief description of what the PR does
- **Motivation**: Why this change is needed
- **Changes**: List of files modified and key changes
- **Testing**: How you tested the changes
- **Breaking Changes**: Any breaking changes (if applicable)

## Project Structure

```
ipv6ddns/
├── src/
│   ├── main.rs          # Main daemon entry point
│   ├── cloudflare.rs    # Cloudflare API client
│   └── netlink.rs       # IPv6 address monitoring
├── android/             # Android companion app
├── packaging/           # Packaging scripts
├── scripts/             # Build and CI scripts
├── etc/                 # Configuration files
└── .github/             # GitHub Actions workflows
```

## Testing

### Unit Tests

Write unit tests for individual functions and modules:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function() {
        let result = function_to_test();
        assert_eq!(result, expected_value);
    }
}
```

### Integration Tests

For integration tests, create a `tests/` directory and add test files:

```rust
// tests/integration_test.rs
use ipv6ddns::Config;

#[test]
fn test_config_load() {
    let config = Config::load(None).unwrap();
    assert!(!config.api_token.is_empty());
}
```

### Testing Guidelines

#### General Principles

- **Test what matters**: Focus on testing business logic, error handling, and edge cases
- **Keep tests independent**: Each test should be able to run in isolation
- **Use descriptive test names**: Test names should clearly describe what they test
- **Test both success and failure paths**: Ensure error cases are properly handled

#### Rust Testing

- Use `#[cfg(test)]` for test modules
- Use `serial_test` for tests that require exclusive access to shared resources (e.g., environment variables)
- Use `tempfile` for creating temporary files in tests
- Test public APIs with realistic inputs and edge cases
- Use `anyhow` context in error assertions for better debugging

```rust
#[test]
#[serial]
fn test_config_with_env_override() {
    let _env = EnvGuard::new();
    std::env::set_var("CLOUDFLARE_API_TOKEN", "test_token");
    let config = Config::load(None).unwrap();
    assert_eq!(config.api_token, "test_token");
}
```

#### Android Testing

- Unit tests should be in the standard `test` source set
- Use JUnit 4 assertions and test annotations
- Test data classes and business logic in isolation
- Mock dependencies where appropriate

```kotlin
@Test
fun testConfigValidation() {
    val config = AppConfig(
        apiToken = "test_token",
        zoneId = "test_zone",
        recordName = "test.example.com"
    )
    assertTrue(config.apiToken.isNotEmpty())
}
```

#### Test Coverage

- Aim for high coverage of core business logic
- Focus on critical paths: config loading, API interactions, state management
- Test error handling and edge cases thoroughly
- Document any untestable code with comments explaining why

#### Running Tests

```bash
# Run all Rust tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_function_name

# Run tests for a specific module
cargo test cloudflare
cargo test netlink

# Run Android tests
./gradlew -p android test

# Run tests in CI
# Tests are automatically run in CI on every PR
```

## Security Considerations

- Never commit secrets or API tokens
- Use environment variables for sensitive data
- Validate all user input
- Follow OWASP guidelines for security
- Run security scanning tools (cargo-audit, cargo-deny)

## Getting Help

- Open an issue for bugs or feature requests
- Ask questions in discussions
- Check existing documentation
- Review existing code for examples

## License

By contributing to this project, you agree that your contributions will be licensed under the MIT License.

## Thank You

Thank you for contributing to ipv6ddns! Your contributions help make this project better for everyone.
