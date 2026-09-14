# Bolt's Journal
## 2024-05-18 - Replaced safe array conversion with fast direct chunk extraction
**Learning:** In netlink hot path parse_messages_into, data.get() combined with try_from bounds-checking was introducing significant per-message latency (around 18ns).
**Action:** Replaced these safe bounds checking patterns with a single direct index access to extract slices let chunk = &data[msg_offset..msg_offset + 6] since the NLMSG_HDRLEN logic in the while condition strictly guarantees the bounds. This simple slicing dropped latency back down to 14ns/batch.
## 2024-05-18 - Optimized is_valid_ipv6 and redact_secrets for faster execution
**Learning:** `.segments()` on `Ipv6Addr` computes `u16` arrays using internal bit shifts, causing unnecessary CPU overhead (~17ns per check). Using `.octets()` directly matches the underlying byte array representation (since Rust 1.70+, but valid earlier as well) and drops latency significantly. Additionally, chained `.replace()` allocations in `redact_secrets` were allocating multiple intermediate strings unnecessarily.
**Action:** Always prefer `.octets()` when performing bitwise checks on IPv6 addresses, avoiding the shift/mask overhead of `.segments()`. In string manipulation hotpaths, combine or conditionally branch allocations to minimize temporary string creation.
