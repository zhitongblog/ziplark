# Third-Party Licenses

Ziplark itself is licensed under the [MIT License](LICENSE), Copyright © 2026
doaipm. It links and bundles third-party open-source components, each under its
own license, acknowledged below.

## UnRAR (RAR / RAR5 extraction)

RAR archive **extraction** is provided by the **UnRAR** library (via the
`unrar` / `unrar_sys` crates, which vendor the UnRAR C++ source). UnRAR is **not**
covered by Ziplark's MIT license — it is distributed under the **UnRAR license**,
whose key terms are:

> The UnRAR sources may be used in any software to handle RAR archives without
> limitations free of charge, but **cannot be used to develop RAR (WinRAR)
> compatible archiver and to re-create the RAR compression algorithm, which is
> proprietary.** Distribution of modified UnRAR sources in separate form or as a
> part of other software is permitted, provided that the full text of this
> paragraph, starting from "The UnRAR sources", is included.

Ziplark uses UnRAR **only to extract** RAR archives; it does not, and cannot, be
used to create RAR archives or re-create the RAR compression algorithm.
UnRAR © Alexander Roshal. See <https://www.rarlab.com/rar_add.htm>.

## Bundled Rust crates

The following crates are compiled into Ziplark. All are permissive or
weak-copyleft (MPL-2.0) and GPL/MIT-compatible. Where a crate offers a choice of
licenses (e.g. "MIT OR Apache-2.0"), Ziplark uses it under those terms; the full
license text of each is available in the crate's source on crates.io / its
repository.

**(MIT OR Apache-2.0) AND Unicode-3.0**
`unicode-ident 1.0.24`

**0BSD OR MIT OR Apache-2.0**
`adler2 2.0.1`

**Apache-2.0**
`lzma-rust2 0.2.2`, `sevenz-rust2 0.13.2`, `sync_wrapper 1.0.2`, `tao 0.35.3`, `zopfli 0.8.3`

**Apache-2.0 / MIT**
`fnv 1.0.7`

**Apache-2.0 AND MIT**
`dpi 0.1.2`

**Apache-2.0 OR MIT**
`atomic-waker 1.1.2`, `autocfg 1.5.1`, `bit-set 0.8.0`, `bit-vec 0.8.0`, `cargo_toml 0.22.3`, `ctor 0.8.0`, `ctor-proc-macro 0.0.7`, `dtor 0.3.0`, `dtor-proc-macro 0.0.6`, `equivalent 1.0.2`, `fastrand 2.4.1`, `idna_adapter 1.2.2`, `indexmap 1.9.3`, `indexmap 2.14.0`, `libappindicator 0.9.0`, `libappindicator-sys 0.9.0`, `muda 0.19.2`, `nt-time 0.11.2`, `pin-project-lite 0.2.17`, `portable-atomic 1.13.1`, `portable-atomic-util 0.2.7`, `rustc-hash 2.1.2`, `tauri 2.11.2`, `tauri-build 2.6.2`, `tauri-codegen 2.6.2`, `tauri-macros 2.6.2`, `tauri-plugin 2.6.2`, `tauri-plugin-dialog 2.7.1`, `tauri-plugin-fs 2.5.1`, `tauri-runtime 2.11.2`, `tauri-runtime-wry 2.11.2`, `tauri-utils 2.9.2`, `utf8_iter 1.0.4`, `uuid 1.23.3`, `window-vibrancy 0.6.0`, `wry 0.55.1`, `zeroize 1.9.0`, `zeroize_derive 1.5.0`

**Apache-2.0 WITH LLVM-exception**
`target-lexicon 0.12.16`

**Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT**
`linux-raw-sys 0.12.1`, `rustix 1.1.4`, `wasi 0.11.1+wasi-snapshot-preview1`, `wasip2 1.0.4+wasi-0.2.12`, `wasip3 0.4.0+wasi-0.3.0-rc-2026-01-06`, `wasm-encoder 0.244.0`, `wasm-metadata 0.244.0`, `wasmparser 0.244.0`, `wit-bindgen 0.51.0`, `wit-bindgen 0.57.1`, `wit-bindgen-core 0.51.0`, `wit-bindgen-rust 0.51.0`, `wit-bindgen-rust-macro 0.51.0`, `wit-component 0.244.0`, `wit-parser 0.244.0`

**Apache-2.0/MIT**
`cesu8 1.1.0`, `dbus 0.9.11`, `libdbus-sys 0.2.7`

**BSD-2-Clause OR Apache-2.0 OR MIT**
`zerocopy 0.8.52`, `zerocopy-derive 0.8.52`

**BSD-3-Clause**
`alloc-no-stdlib 2.0.4`, `alloc-stdlib 0.2.4`, `subtle 2.6.1`

**BSD-3-Clause AND MIT**
`brotli 8.0.4`

**BSD-3-Clause OR MIT OR Apache-2.0**
`num_enum 0.7.6`, `num_enum_derive 0.7.6`

**BSD-3-Clause/MIT**
`brotli-decompressor 5.0.3`

**CC0-1.0 OR MIT-0 OR Apache-2.0**
`constant_time_eq 0.3.1`, `dunce 1.0.5`

**ISC**
`libloading 0.7.4`

**MIT**
`atk 0.18.2`, `atk-sys 0.18.2`, `block2 0.6.2`, `bytes 1.11.1`, `cairo-rs 0.18.5`, `cairo-sys-rs 0.18.2`, `cargo_metadata 0.19.2`, `cfb 0.7.3`, `combine 4.6.7`, `darling 0.23.0`, `darling_core 0.23.0`, `darling_macro 0.23.0`, `derive_more 2.1.1`, `derive_more-impl 2.1.1`, `dlopen2 0.8.2`, `dlopen2_derive 0.4.3`, `dom_query 0.27.0`, `embed-resource 3.0.9`, `gdk 0.18.2`, `gdk-pixbuf 0.18.5`, `gdk-pixbuf-sys 0.18.0`, `gdk-sys 0.18.2`, `gdkwayland-sys 0.18.2`, `gdkx11 0.18.2`, `gdkx11-sys 0.18.2`, `generic-array 0.14.7`, `gio 0.18.4`, `gio-sys 0.18.1`, `glib 0.18.5`, `glib-macros 0.18.5`, `glib-sys 0.18.1`, `gobject-sys 0.18.0`, `gtk 0.18.2`, `gtk-sys 0.18.2`, `gtk3-macros 0.18.2`, `http-body 1.0.1`, `http-body-util 0.1.3`, `hyper 1.10.1`, `hyper-util 0.1.20`, `ico 0.5.0`, `infer 0.19.0`, `javascriptcore-rs 1.1.2`, `javascriptcore-rs-sys 1.1.1`, `libredox 0.1.17`, `lz4_flex 0.11.6`, `lzma-rs 0.3.0`, `memoffset 0.9.1`, `mio 1.2.1`, `new_debug_unreachable 1.0.6`, `objc2 0.6.4`, `objc2-encode 4.1.0`, `objc2-foundation 0.3.2`, `pango 0.18.3`, `pango-sys 0.18.0`, `phf 0.13.1`, `phf_codegen 0.13.1`, `phf_generator 0.13.1`, `phf_macros 0.13.1`, `phf_shared 0.13.1`, `plist 1.9.0`, `precomputed-hash 0.1.1`, `quick-xml 0.39.4`, `redox_syscall 0.5.18`, `redox_users 0.5.2`, `rfd 0.16.0`, `schemars 0.8.22`, `schemars 0.9.0`, `schemars 1.2.1`, `schemars_derive 0.8.22`, `simd-adler32 0.3.9`, `slab 0.4.12`, `soup3 0.5.0`, `soup3-sys 0.5.0`, `strsim 0.11.1`, `synstructure 0.13.2`, `tauri-winres 0.3.6`, `tokio 1.52.3`, `tokio-util 0.7.18`, `tower 0.5.3`, `tower-http 0.6.11`, `tower-layer 0.3.3`, `tower-service 0.3.3`, `tracing 0.1.44`, `tracing-core 0.1.36`, `try-lock 0.2.5`, `twox-hash 2.1.2`, `urlpattern 0.3.0`, `version-compare 0.2.1`, `vswhom 0.1.0`, `vswhom-sys 0.1.3`, `want 0.3.1`, `webkit2gtk 2.0.2`, `webkit2gtk-sys 2.0.2`, `webview2-com 0.38.2`, `webview2-com-macros 0.8.1`, `webview2-com-sys 0.38.2`, `winnow 0.5.40`, `winnow 0.7.15`, `winnow 1.0.3`, `winreg 0.55.0`, `x11 2.21.0`, `x11-dl 2.21.0`, `zip 2.4.2`, `zmij 1.0.21`, `zstd 0.13.3`

**MIT OR Apache-2.0**
`aes 0.8.4`, `anyhow 1.0.102`, `arbitrary 1.4.2`, `base64 0.21.7`, `base64 0.22.1`, `bitflags 2.13.0`, `block-buffer 0.10.4`, `block-padding 0.3.3`, `bumpalo 3.20.3`, `bzip2 0.5.2`, `camino 1.2.2`, `cargo-platform 0.1.9`, `cbc 0.1.2`, `cc 1.2.64`, `cfg-expr 0.15.8`, `cfg-if 1.0.4`, `chrono 0.4.45`, `cipher 0.4.4`, `cookie 0.18.1`, `core-foundation 0.10.1`, `core-foundation-sys 0.8.7`, `core-graphics 0.25.0`, `core-graphics-types 0.2.0`, `cpufeatures 0.2.17`, `crc 3.4.0`, `crc-catalog 2.5.0`, `crc32fast 1.5.0`, `crossbeam-channel 0.5.15`, `crossbeam-utils 0.8.21`, `crypto-common 0.1.7`, `deranged 0.5.8`, `derive_arbitrary 1.4.2`, `digest 0.10.7`, `dirs 6.0.0`, `dirs-sys 0.5.0`, `displaydoc 0.2.6`, `dtoa 1.0.11`, `dyn-clone 1.0.20`, `embed_plist 1.2.2`, `erased-serde 0.4.10`, `errno 0.3.14`, `fdeflate 0.3.7`, `field-offset 0.3.6`, `find-msvc-tools 0.1.9`, `flate2 1.1.9`, `form_urlencoded 1.2.2`, `futures-channel 0.3.32`, `futures-core 0.3.32`, `futures-executor 0.3.32`, `futures-io 0.3.32`, `futures-macro 0.3.32`, `futures-sink 0.3.32`, `futures-task 0.3.32`, `futures-util 0.3.32`, `getrandom 0.2.17`, `getrandom 0.3.4`, `getrandom 0.4.2`, `glob 0.3.3`, `hashbrown 0.12.3`, `hashbrown 0.15.5`, `hashbrown 0.17.1`, `heck 0.4.1`, `heck 0.5.0`, `hex 0.4.3`, `hmac 0.12.1`, `html5ever 0.38.0`, `http 1.4.2`, `httparse 1.10.1`, `iana-time-zone 0.1.65`, `iana-time-zone-haiku 0.1.2`, `idna 1.1.0`, `inout 0.1.4`, `ipnet 2.12.0`, `itoa 1.0.18`, `jni-sys 0.3.1`, `jni-sys 0.4.1`, `jni-sys-macros 0.4.1`, `jobserver 0.1.34`, `js-sys 0.3.102`, `jsonptr 0.6.3`, `keyboard-types 0.7.0`, `leb128fmt 0.1.0`, `libc 0.2.186`, `lock_api 0.4.14`, `log 0.4.32`, `markup5ever 0.38.0`, `mime 0.3.17`, `ndk 0.9.0`, `ndk-sys 0.6.0+11769913`, `num-conv 0.2.2`, `num-traits 0.2.19`, `once_cell 1.21.4`, `parking_lot 0.12.5`, `parking_lot_core 0.9.12`, `pbkdf2 0.12.2`, `percent-encoding 2.3.2`, `pkg-config 0.3.33`, `png 0.17.16`, `png 0.18.1`, `powerfmt 0.2.0`, `ppv-lite86 0.2.21`, `prettyplease 0.2.37`, `proc-macro-crate 1.3.1`, `proc-macro-crate 2.0.2`, `proc-macro-crate 3.5.0`, `proc-macro-error 1.0.4`, `proc-macro-error-attr 1.0.4`, `proc-macro2 1.0.106`, `quote 1.0.45`, `rand 0.9.4`, `rand_chacha 0.9.0`, `rand_core 0.9.5`, `ref-cast 1.0.25`, `ref-cast-impl 1.0.25`, `regex 1.12.4`, `regex-automata 0.4.14`, `regex-syntax 0.8.11`, `reqwest 0.13.4`, `rustc_version 0.4.1`, `rustversion 1.0.22`, `scopeguard 1.2.0`, `semver 1.0.28`, `serde 1.0.228`, `serde-untagged 0.1.9`, `serde_core 1.0.228`, `serde_derive 1.0.228`, `serde_derive_internals 0.29.1`, `serde_json 1.0.150`, `serde_repr 0.1.20`, `serde_spanned 0.6.9`, `serde_spanned 1.1.1`, `serde_with 3.21.0`, `serde_with_macros 3.21.0`, `serialize-to-javascript 0.1.2`, `serialize-to-javascript-impl 0.1.2`, `servo_arc 0.4.3`, `sha1 0.10.6`, `sha2 0.10.9`, `shlex 2.0.1`, `smallvec 1.15.2`, `socket2 0.6.4`, `softbuffer 0.4.8`, `stable_deref_trait 1.2.1`, `string_cache 0.9.0`, `string_cache_codegen 0.6.1`, `swift-rs 1.0.7`, `syn 1.0.109`, `syn 2.0.117`, `system-deps 6.2.2`, `tao-macros 0.1.3`, `tar 0.4.46`, `tendril 0.5.0`, `thiserror 1.0.69`, `thiserror 2.0.18`, `thiserror-impl 1.0.69`, `thiserror-impl 2.0.18`, `time 0.3.49`, `time-core 0.1.9`, `time-macros 0.2.29`, `toml 0.8.2`, `toml 0.9.12+spec-1.1.0`, `toml 1.1.2+spec-1.1.0`, `toml_datetime 0.6.3`, `toml_datetime 0.7.5+spec-1.1.0`, `toml_datetime 1.1.1+spec-1.1.0`, `toml_edit 0.19.15`, `toml_edit 0.20.2`, `toml_edit 0.25.12+spec-1.1.0`, `toml_parser 1.1.2+spec-1.1.0`, `toml_writer 1.1.1+spec-1.1.0`, `tray-icon 0.23.1`, `typeid 1.0.3`, `typenum 1.20.1`, `unicode-segmentation 1.13.3`, `unicode-xid 0.2.6`, `unrar 0.5.8`, `unrar_sys 0.5.8`, `url 2.5.8`, `utf-8 0.7.6`, `wasm-bindgen 0.2.125`, `wasm-bindgen-futures 0.4.75`, `wasm-bindgen-macro 0.2.125`, `wasm-bindgen-macro-support 0.2.125`, `wasm-bindgen-shared 0.2.125`, `wasm-streams 0.5.0`, `web-sys 0.3.102`, `web_atoms 0.2.4`, `widestring 1.2.1`, `windows 0.61.3`, `windows-collections 0.2.0`, `windows-core 0.61.2`, `windows-core 0.62.2`, `windows-future 0.2.1`, `windows-implement 0.60.2`, `windows-interface 0.59.3`, `windows-link 0.1.3`, `windows-link 0.2.1`, `windows-numerics 0.2.0`, `windows-result 0.3.4`, `windows-result 0.4.1`, `windows-strings 0.4.2`, `windows-strings 0.5.1`, `windows-sys 0.45.0`, `windows-sys 0.52.0`, `windows-sys 0.59.0`, `windows-sys 0.60.2`, `windows-sys 0.61.2`, `windows-targets 0.42.2`, `windows-targets 0.52.6`, `windows-targets 0.53.5`, `windows-threading 0.1.0`, `windows-version 0.1.7`, `windows_aarch64_gnullvm 0.42.2`, `windows_aarch64_gnullvm 0.52.6`, `windows_aarch64_gnullvm 0.53.1`, `windows_aarch64_msvc 0.42.2`, `windows_aarch64_msvc 0.52.6`, `windows_aarch64_msvc 0.53.1`, `windows_i686_gnu 0.42.2`, `windows_i686_gnu 0.52.6`, `windows_i686_gnu 0.53.1`, `windows_i686_gnullvm 0.52.6`, `windows_i686_gnullvm 0.53.1`, `windows_i686_msvc 0.42.2`, `windows_i686_msvc 0.52.6`, `windows_i686_msvc 0.53.1`, `windows_x86_64_gnu 0.42.2`, `windows_x86_64_gnu 0.52.6`, `windows_x86_64_gnu 0.53.1`, `windows_x86_64_gnullvm 0.42.2`, `windows_x86_64_gnullvm 0.52.6`, `windows_x86_64_gnullvm 0.53.1`, `windows_x86_64_msvc 0.42.2`, `windows_x86_64_msvc 0.52.6`, `windows_x86_64_msvc 0.53.1`, `xattr 1.6.1`, `zstd-safe 7.2.4`

**MIT OR Apache-2.0 OR LGPL-2.1-or-later**
`r-efi 5.3.0`, `r-efi 6.0.0`

**MIT OR Apache-2.0 OR Zlib**
`raw-window-handle 0.6.2`, `tinyvec_macros 0.1.1`

**MIT OR Zlib OR Apache-2.0**
`miniz_oxide 0.8.9`

**MIT/Apache-2.0**
`android_system_properties 0.1.5`, `bitflags 1.3.2`, `bs58 0.5.1`, `bzip2 0.4.4`, `bzip2-sys 0.1.13+1.0.8`, `filetime 0.2.29`, `filetime_creation 0.2.0`, `foreign-types 0.5.0`, `foreign-types-macros 0.2.3`, `foreign-types-shared 0.3.1`, `id-arena 2.3.0`, `ident_case 1.0.1`, `jni 0.21.1`, `json-patch 3.0.1`, `lzma-sys 0.1.20`, `siphasher 1.0.3`, `unic-char-property 0.9.0`, `unic-char-range 0.9.0`, `unic-common 0.9.0`, `unic-ucd-ident 0.9.0`, `unic-ucd-version 0.9.0`, `version_check 0.9.5`, `winapi 0.3.9`, `winapi-i686-pc-windows-gnu 0.4.0`, `winapi-x86_64-pc-windows-gnu 0.4.0`, `xz2 0.1.7`, `zstd-sys 2.0.16+zstd.1.5.7`

**MPL-2.0**
`cssparser 0.36.0`, `cssparser-macros 0.6.1`, `dtoa-short 0.3.5`, `option-ext 0.2.0`, `selectors 0.36.1`

**Unicode-3.0**
`icu_collections 2.2.0`, `icu_locale_core 2.2.0`, `icu_normalizer 2.2.0`, `icu_normalizer_data 2.2.0`, `icu_properties 2.2.0`, `icu_properties_data 2.2.0`, `icu_provider 2.2.0`, `litemap 0.8.2`, `potential_utf 0.1.5`, `tinystr 0.8.3`, `writeable 0.6.3`, `yoke 0.8.3`, `yoke-derive 0.8.2`, `zerofrom 0.1.8`, `zerofrom-derive 0.1.7`, `zerotrie 0.2.4`, `zerovec 0.11.6`, `zerovec-derive 0.11.3`

**Unlicense OR MIT**
`aho-corasick 1.1.4`, `byteorder 1.5.0`, `jiff 0.2.28`, `jiff-static 0.2.28`, `memchr 2.8.2`, `winapi-util 0.1.11`

**Unlicense/MIT**
`same-file 1.0.6`, `walkdir 2.5.0`

**Zlib**
`foldhash 0.1.5`, `foldhash 0.2.0`

**Zlib OR Apache-2.0 OR MIT**
`bytemuck 1.25.0`, `dispatch2 0.3.1`, `objc2-app-kit 0.3.2`, `objc2-cloud-kit 0.3.2`, `objc2-core-data 0.3.2`, `objc2-core-foundation 0.3.2`, `objc2-core-graphics 0.3.2`, `objc2-core-image 0.3.2`, `objc2-core-location 0.3.2`, `objc2-core-text 0.3.2`, `objc2-exception-helper 0.1.1`, `objc2-io-surface 0.3.2`, `objc2-quartz-core 0.3.2`, `objc2-ui-kit 0.3.2`, `objc2-user-notifications 0.3.2`, `objc2-web-kit 0.3.2`, `tinyvec 1.11.0`

_Total: 502 third-party crates._

## Notes

- **Apache-2.0** components: their `NOTICE` files (where present) are preserved
  in the respective crate sources.
- **MPL-2.0** components (e.g. `cssparser`, `selectors`, from the Tauri webview
  stack) are used unmodified; their source is available on crates.io and the
  Servo project repositories.
- Full, verbatim license texts for every crate are retrievable from
  <https://crates.io> (each crate page links its license), or via
  `cargo about generate` against this repository.
