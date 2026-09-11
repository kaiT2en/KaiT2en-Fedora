# t2-biometrickit

BiometricKit protocol for the Apple T2 Touch ID sensor.

The sensor hangs on the SEP endpoint `sbio` inside bridgeOS and is reachable
only over BridgeXPC on the CDC-NCM link, never through the host's SEP mailbox.
This crate speaks that protocol on top of `t2-bridgexpc`. It needs no kernel
module of ours beyond the BCE stack that carries the link.

It makes no policy decisions. It reports which enrolled finger matched; whether
that logs anyone in is decided a layer up.

```rust
let mut session = Session::open(None, None)?;
session.read_identities(502)?;
session.start_match(0, proto::NO_CREDENTIAL_SET)?;
while let Some(event) = session.next_event(Duration::from_secs(30))? {
    if let Event::MatchResult { slot, .. } = event { /* … */ }
}
```

`Session::open` discovers the link, activates the service, resets the sensor and
loads the FDR calibration. Without that calibration the sensor stays dark, so it
is part of opening rather than a separate step.

`start_match` takes the processed match flags and a credential-set handle. Use
`proto::NO_CREDENTIAL_SET` for a plain verify: a handle bridgeOS cannot resolve
makes it declare a match lockout and pause the capture, so no image is ever
processed. Flags `0` is the plain verify and needs no bound credential set and
no acmContext.

No enrollment. The finger is enrolled under macOS; this crate only verifies
against it.
