// ARM64 Bionic requires an executable's PT_TLS segment to be aligned to at
// least 64 bytes. Gullicube's TLS data otherwise only requests 8-byte alignment.
alignas(64) thread_local unsigned char bionic_tls_alignment;
