# ippper

[![crates.io](https://img.shields.io/crates/v/ippper.svg)](https://crates.io/crates/ippper)
[![Released API docs](https://docs.rs/ippper/badge.svg)](https://docs.rs/ippper)
[![BSD 3 Clause licensed](https://img.shields.io/badge/license-BSD%203%20Clause-blue)](./LICENSE.md)

A library for building simple ipp servers.  
A **very basic** implementation of ipp is provided as `ippper::service::SimpleIppService`. This is very basic, but enough to fool Android Printer Service & Windows Print Spooler.

- Easy to use
- Asynchronous design

## "Low-level"
ippper is a relatively low-level library.

## Adaptive HTTP/HTTPS
With the `server-tls` feature, `server::serve_adaptive_https` serves plain HTTP
and HTTPS on the same port. Plain HTTP clients may also switch to TLS 1.2 or 1.3
on an existing connection, as described in
[RFC 2817](https://www.rfc-editor.org/rfc/rfc2817).

Note that the request carrying the upgrade is sent in plaintext. To protect a
print job, upgrade with `OPTIONS *` first, then send the job over TLS.

## License
Licensed under [BSD 3 Clause](./LICENSE.md)
