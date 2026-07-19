# Logitech G560 zone map

## Device evidence

- USB ID: `046d:0a78`
- Reported device revision: `90.64`
- USB interfaces 0 and 1 are audio interfaces driven by `snd-usb-audio`.
- USB interface 2 is a HID interface driven by `usbhid`; the driver validates that every alternate descriptor for this interface is HID before detaching it.

## Physical mapping

The protocol-to-physical mapping below was observed directly on the connected speakers while audio played.

| Protocol index | Observed speaker | Observed light | Verification |
| --- | --- | --- | --- |
| `0x00` | left | front | white pulse observed; returned black |
| `0x01` | right | front | white pulse observed; returned black |
| `0x02` | left | rear | white pulse observed; returned black |
| `0x03` | right | rear | white pulse observed; returned black |

### Observation sequence

Quiet audio played through the G560 throughout verification. Each raw index was set to white for 12 seconds and then returned to black. The longer window made it possible to identify widely separated speakers reliably. Testing stopped between pulses until the observer explicitly confirmed that the selected zone returned black and audio remained uninterrupted.

The verified logical mapping is:

- logical left rear → `0x02`
- logical left front → `0x00`
- logical right front → `0x01`
- logical right rear → `0x03`

All four pulses completed with uninterrupted audio. An initial unpaced multi-report blackout accepted its first report and rejected its second with a USB I/O error. Persistent control therefore spaces every adjacent report by 20 ms, including reports separated by public API calls; it never detaches between reports. A five-minute soak rotated red, green, blue, and white across all four logical zones without a USB error or audio interruption, then returned every zone to black.
