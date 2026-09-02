# i915-psr

Test v2 of the upstream i915 patch enabling Panel Self Refresh on Apple T2
eDP panels.

The patch implements the Apple TCON setup and capture-trigger sequence as an
i915 DPCD quirk. It is restricted to Apple-OUI eDP sinks on systems containing
an Apple T2; Apple panels on other systems retain the existing PSR block.

The primary risks are internal-panel flicker or blanking and regressions across
suspend and resume. A successful test should show the Apple PSR handshake quirk
in the kernel log and the sink entering PSR1, including after suspend/resume.

The patch is the v2 submission generated against drm-tip base commit
`7f9a02c0a1d05882e8625741defa9e088bd55791`.
