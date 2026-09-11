# t2-touchid-probe

BridgeXPC/BiometricKit probe for the Apple T2 Touch ID sensor. It finds the
Apple CDC-NCM link, activates `com.apple.eos.BiometricKit` over RemoteXPC,
loads the sensor calibration, and runs either a presence scan or a verify-only
match against the fingers already enrolled on this machine.

It never enrolls, and it never touches `/dev/t2sep`: the biometric path and the
keybag path are separate. Its output must not be used as an authentication
decision.

## Status: not installed

`install-apps.sh` does not install it. This is the research tool that
established the protocol; the protocol itself is what moves to Rust.

```text
make
./t2-touchid-probe -m -f 0 -u 502 -t 30
```

The interface and the BiometricKit port are discovered. `-t` sets the
observation window, 1 to 300 seconds. `-u` is the macOS user ID whose identity
inventory is read with command `0x42`; only record counts, user IDs and the
first four UUID bytes are printed.

## matchInit

The match request is a 68 byte matchInit, optionally followed by 20 byte
identity records. The driver copies its first eight bytes as one quadword, so:

```text
+0  u32  processed match flags   -> 0x1da8   (-f)
+4  u32  credential-set handle   -> 0x1dac   (-k)
+8  u32  acmContext length       -> 0x1de4
+12  …   acmContext              -> 0x1dc4
```

The credential-set handle must be `0xffffffff` when no credential set is bound,
which is the default. A value the driver cannot resolve makes
`checkAutoMatchLockoutConditions` report a lockout, and `sensorStateHandler`
then pauses the capture as soon as the finger goes down, so no image is
processed and no result is produced.

Flag bit 0 requires a bound credential set, bit 1 extends an enrollment, bit 3
carries an acmContext and bit 4 has the SEP validate it. Bit 8 is standalone
and excludes the others. `-f 0` is the plain verify and needs none of them.

Status 63 is finger-down and 64 finger-up on the tested firmware. Other codes
are printed numerically without interpretation.
