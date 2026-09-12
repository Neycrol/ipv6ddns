# Bolt's Journal
## 2024-05-18 - Replaced safe array conversion with fast direct chunk extraction
**Learning:** In netlink hot path parse_messages_into, data.get() combined with try_from bounds-checking was introducing significant per-message latency (around 18ns).
**Action:** Replaced these safe bounds checking patterns with a single direct index access to extract slices let chunk = &data[msg_offset..msg_offset + 6] since the NLMSG_HDRLEN logic in the while condition strictly guarantees the bounds. This simple slicing dropped latency back down to 14ns/batch.

## 2025-02-18 - Replacing vec! with Array for Netlink Dumps
**Learning:** In `netlink_dump_ipv6`, a `vec![0u8; NETLINK_DUMP_BUFFER_SIZE]` (16KB) was allocated for receiving netlink responses. Since 16KB easily fits on a standard thread stack and this function is run every `poll_interval` when in polling mode (e.g. containers lacking netlink events), the heap allocation adds up unnecessarily over time.
**Action:** Replace `vec![0u8; SIZE]` with `[0u8; SIZE]` on the hot/polling path to safely eliminate heap allocations where the buffer size is statically known and moderate in size.
