//! IPv6 validation benchmarks
//!
//! This benchmark suite measures the performance of IPv6 address validation
//! and DNS record validation functions. It includes both success and rejection
//! path benchmarks to ensure error handling doesn't introduce performance regressions.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ipv6ddns::validation::{is_valid_ipv6, validate_record_name};

/// Performance thresholds (in microseconds)
/// These values define the maximum acceptable time for each operation
const IPV6_VALIDATION_THRESHOLD_US: u64 = 5; // IPv6 validation should be < 5μs
const DNS_VALIDATION_THRESHOLD_US: u64 = 10; // DNS validation should be < 10μs
const REJECTION_PATH_MULTIPLIER: f64 = 2.0; // Rejection paths should be < 2x success path

/// Benchmark IPv6 validation with various input types
fn bench_ipv6_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipv6_validation");

    // Valid IPv6 addresses
    let valid_ips = vec![
        "2001:db8::1",
        "::1",
        "fe80::1",
        "2001:0db8:0000:0000:0000:0000:0000:0001",
    ];

    // Invalid IPv6 addresses (rejection paths)
    let invalid_ips = vec![
        "192.168.1.1", // IPv4
        "invalid",     // Not an IP
        "",            // Empty
        "2001:db8::g", // Invalid character
        ":::1",        // Too many colons
    ];

    // Benchmark valid IPv6 addresses (success path)
    for ip in &valid_ips {
        group.bench_with_input(BenchmarkId::new("valid", ip), ip, |b, ip| {
            b.iter(|| black_box(is_valid_ipv6(black_box(ip), false)))
        });
    }

    // Benchmark invalid IPv6 addresses (rejection path)
    for ip in &invalid_ips {
        group.bench_with_input(BenchmarkId::new("invalid", ip), ip, |b, ip| {
            b.iter(|| black_box(is_valid_ipv6(black_box(ip), false)))
        });
    }

    // Benchmark with loopback allowed
    group.bench_function("loopback_allowed", |b| {
        b.iter(|| black_box(is_valid_ipv6(black_box("::1"), true)))
    });

    // Benchmark with loopback disallowed (rejection path)
    group.bench_function("loopback_disallowed", |b| {
        b.iter(|| black_box(is_valid_ipv6(black_box("::1"), false)))
    });

    group.finish();
}

/// Benchmark DNS record name validation
fn bench_dns_record_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("dns_record_validation");

    // Valid DNS record names
    let valid_records = vec![
        "example.com",
        "subdomain.example.com",
        "deep.nested.subdomain.example.com",
        "test-record.example.com",
    ];

    // Invalid DNS record names (rejection paths)
    let invalid_records = vec![
        "",              // Empty
        "-invalid.com",  // Starts with hyphen
        "invalid-.com",  // Ends with hyphen
        "invalid..com",  // Double dot
        "invalid@.com",  // Invalid character
        "a".repeat(256), // Too long
    ];

    // Benchmark valid DNS record names (success path)
    for record in &valid_records {
        group.bench_with_input(BenchmarkId::new("valid", record), record, |b, record| {
            b.iter(|| black_box(validate_record_name(black_box(record))))
        });
    }

    // Benchmark invalid DNS record names (rejection path)
    for record in &invalid_records {
        // Skip empty string for ID generation
        let id = if record.is_empty() {
            "empty"
        } else {
            &record[..record.len().min(20)]
        };
        group.bench_with_input(BenchmarkId::new("invalid", id), record, |b, record| {
            b.iter(|| black_box(validate_record_name(black_box(record))))
        });
    }

    group.finish();
}

/// Benchmark edge cases and boundary conditions
fn bench_edge_cases(c: &mut Criterion) {
    let mut group = c.benchmark_group("edge_cases");

    // Maximum compression IPv6
    group.bench_function("max_compression", |b| {
        b.iter(|| black_box(is_valid_ipv6(black_box("::"), false)))
    });

    // Full IPv6 address (no compression)
    group.bench_function("no_compression", |b| {
        b.iter(|| {
            black_box(is_valid_ipv6(
                black_box("2001:0db8:0000:0000:0000:0000:0000:0001"),
                false,
            ))
        })
    });

    // IPv6 with zone ID (should be rejected)
    group.bench_function("zone_id_rejection", |b| {
        b.iter(|| black_box(is_valid_ipv6(black_box("fe80::1%eth0"), false)))
    });

    // Single label DNS record
    group.bench_function("single_label_dns", |b| {
        b.iter(|| black_box(validate_record_name(black_box("localhost"))))
    });

    group.finish();
}

/// Benchmark throughput - measure operations per second
fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("throughput");
    group.throughput(criterion::Throughput::Elements(1000));

    // Batch validation of IPv6 addresses
    let batch_ips: Vec<&str> = (0..1000)
        .map(|i| match i % 4 {
            0 => "2001:db8::1",
            1 => "::1",
            2 => "fe80::1",
            _ => "2001:0db8::1",
        })
        .collect();

    group.bench_function("batch_ipv6_validation", |b| {
        b.iter(|| {
            for ip in &batch_ips {
                black_box(is_valid_ipv6(black_box(ip), false));
            }
        })
    });

    // Batch validation of DNS records
    let batch_records: Vec<&str> = (0..1000)
        .map(|i| match i % 4 {
            0 => "example.com",
            1 => "sub.example.com",
            2 => "deep.nested.example.com",
            _ => "test.example.com",
        })
        .collect();

    group.bench_function("batch_dns_validation", |b| {
        b.iter(|| {
            for record in &batch_records {
                black_box(validate_record_name(black_box(record)));
            }
        })
    });

    group.finish();
}

/// Validate performance thresholds
fn validate_thresholds() {
    use std::time::Instant;

    println!("\n=== Performance Threshold Validation ===\n");

    // Test IPv6 validation threshold
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = is_valid_ipv6("2001:db8::1", false);
    }
    let duration = start.elapsed();
    let avg_time_us = duration.as_micros() as f64 / 1000.0;

    println!("IPv6 validation average time: {:.2}μs", avg_time_us);
    assert!(
        avg_time_us < IPV6_VALIDATION_THRESHOLD_US as f64,
        "IPv6 validation threshold exceeded: {:.2}μs >= {}μs",
        avg_time_us,
        IPV6_VALIDATION_THRESHOLD_US
    );

    // Test DNS validation threshold
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = validate_record_name("example.com");
    }
    let duration = start.elapsed();
    let avg_time_us = duration.as_micros() as f64 / 1000.0;

    println!("DNS validation average time: {:.2}μs", avg_time_us);
    assert!(
        avg_time_us < DNS_VALIDATION_THRESHOLD_US as f64,
        "DNS validation threshold exceeded: {:.2}μs >= {}μs",
        avg_time_us,
        DNS_VALIDATION_THRESHOLD_US
    );

    // Test rejection path performance
    let start = Instant::now();
    for _ in 0..1000 {
        let _ = is_valid_ipv6("invalid", false);
    }
    let rejection_duration = start.elapsed();
    let rejection_avg_us = rejection_duration.as_micros() as f64 / 1000.0;

    println!("Rejection path average time: {:.2}μs", rejection_avg_us);
    assert!(
        rejection_avg_us < avg_time_us * REJECTION_PATH_MULTIPLIER,
        "Rejection path too slow: {:.2}μs >= {:.2}μs",
        rejection_avg_us,
        avg_time_us * REJECTION_PATH_MULTIPLIER
    );

    println!("\n✅ All performance thresholds validated successfully!");
}

// Main benchmark runner
fn run_benchmarks(c: &mut Criterion) {
    bench_ipv6_validation(c);
    bench_dns_record_validation(c);
    bench_edge_cases(c);
    bench_throughput(c);
}

// Test mode for CI validation
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_thresholds() {
        validate_thresholds();
    }

    #[test]
    fn test_rejection_path_performance() {
        use std::time::Instant;

        // Measure success path
        let start = Instant::now();
        for _ in 0..1000 {
            let _ = is_valid_ipv6("2001:db8::1", false);
        }
        let success_time = start.elapsed();

        // Measure rejection path
        let start = Instant::now();
        for _ in 0..1000 {
            let _ = is_valid_ipv6("invalid", false);
        }
        let rejection_time = start.elapsed();

        // Rejection path should not be significantly slower
        let ratio = rejection_time.as_secs_f64() / success_time.as_secs_f64();
        assert!(
            ratio < REJECTION_PATH_MULTIPLIER,
            "Rejection path too slow: ratio {:.2} >= {:.2}",
            ratio,
            REJECTION_PATH_MULTIPLIER
        );
    }
}

criterion_group!(benches, run_benchmarks);
criterion_main!(benches);
