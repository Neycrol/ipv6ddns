## 2024-05-24 - Serde string allocations on temporary structs
**Learning:** Serde native implementations for standard types like `std::net::Ipv6Addr` handle serialization directly to string, allowing zero-allocation serialization when replacing explicit `String` usage with native types. Using lifetimes (`<'a>`) to borrow `&str` on short-lived payload structs further avoids allocations.
**Action:** Avoid allocating `String`s merely to feed them to `serde_json`. Instead, use references (`&str`) and standard types natively inside transient `#[derive(Serialize)]` structs.
