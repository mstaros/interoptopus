# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### ⚠️ Breaking

- **Class-backed enums no longer expose a public parameterless constructor.** `new EnumX()` on a
  class-backed enum previously produced a variant-zero instance from outside the type — a variant
  the Rust side never sent, and one that `default(EnumX)` (a null reference) does not otherwise
  admit. The constructor is now `private`; construct through the generated factories instead.
  Struct-backed enums are unaffected, since their empty state is carried by `_hasValue`.

## [0.16.4](https://github.com/ralfbiedert/interoptopus/compare/interoptopus_csharp-v0.16.3...interoptopus_csharp-v0.16.4)

### 🐛 Bug Fixes


- Prevent async task handle use-after-free - ([b56f758](https://github.com/ralfbiedert/interoptopus/commit/b56f7587acd7fecca4bf69f573cc29d820e9450d))

