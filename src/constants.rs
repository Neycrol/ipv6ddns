//! Common constants used throughout the ipv6ddns application

//==============================================================================
// Cloudflare API Constants
//==============================================================================

/// Cloudflare API base URL
pub const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com/client/v4";

/// User agent string for Cloudflare API requests
pub const CLOUDFLARE_USER_AGENT: &str = "ipv6ddns/1.0";

/// DNS record type for IPv6 addresses
pub const DNS_RECORD_TYPE_AAAA: &str = "AAAA";

/// TTL value for automatic TTL (1 second)
pub const DNS_TTL_AUTO: u64 = 1;

//==============================================================================
// HTTP Status Codes
//==============================================================================

/// HTTP status code for unauthorized requests (401)
pub const HTTP_STATUS_UNAUTHORIZED: u16 = 401;

/// HTTP status code for forbidden requests (403)
pub const HTTP_STATUS_FORBIDDEN: u16 = 403;

/// HTTP status code for rate limiting (429)
pub const HTTP_STATUS_TOO_MANY_REQUESTS: u16 = 429;

/// Minimum HTTP status code for server errors (500)
pub const HTTP_STATUS_SERVER_ERROR_MIN: u16 = 500;

/// Maximum HTTP status code for server errors (599)
pub const HTTP_STATUS_SERVER_ERROR_MAX: u16 = 599;

//==============================================================================
// Timeout and Interval Constants
//==============================================================================

/// Default HTTP request timeout in seconds
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default polling interval in seconds (when netlink is unavailable)
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;

/// Minimum HTTP request timeout in seconds
pub const MIN_TIMEOUT_SECS: u64 = 1;

/// Maximum HTTP request timeout in seconds
pub const MAX_TIMEOUT_SECS: u64 = 300;

/// Minimum polling interval in seconds
pub const MIN_POLL_INTERVAL_SECS: u64 = 10;

/// Maximum polling interval in seconds
pub const MAX_POLL_INTERVAL_SECS: u64 = 3600;

//==============================================================================
// Backoff Constants
//==============================================================================

/// Base delay for exponential backoff in seconds (5 seconds)
pub const BACKOFF_BASE_SECS: u64 = 5;

/// Maximum delay for exponential backoff in seconds (10 minutes)
pub const BACKOFF_MAX_SECS: u64 = 600;

/// Maximum exponent for exponential backoff (capped at 10)
pub const BACKOFF_MAX_EXPONENT: u64 = 10;

//==============================================================================
// Validation Constants
//==============================================================================

/// Minimum API token length in characters
pub const MIN_API_TOKEN_LENGTH: usize = 32;

/// Minimum zone ID length in characters
pub const MIN_ZONE_ID_LENGTH: usize = 16;

/// Maximum zone ID length in characters
pub const MAX_ZONE_ID_LENGTH: usize = 64;

/// Maximum DNS record name length in characters
pub const MAX_RECORD_NAME_LENGTH: usize = 253;

/// Maximum DNS label length in characters
pub const MAX_LABEL_LENGTH: usize = 63;

//==============================================================================
// Environment Variable Names
//==============================================================================

/// Environment variable name for Cloudflare API token
pub const ENV_API_TOKEN: &str = "CLOUDFLARE_API_TOKEN";

/// Environment variable name for Cloudflare zone ID
pub const ENV_ZONE_ID: &str = "CLOUDFLARE_ZONE_ID";

/// Environment variable name for DNS record name
pub const ENV_RECORD_NAME: &str = "CLOUDFLARE_RECORD_NAME";

/// Environment variable name for multi-record policy
pub const ENV_MULTI_RECORD: &str = "CLOUDFLARE_MULTI_RECORD";

/// Environment variable name to allow loopback IPv6 (::1)
pub const ENV_ALLOW_LOOPBACK: &str = "IPV6DDNS_ALLOW_LOOPBACK";
