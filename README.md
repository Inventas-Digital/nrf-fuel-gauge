# nrf-fuel-gauge

Rust FFI wrapper for Nordic Semiconductor's [nRF Fuel Gauge](https://github.com/nrfconnect/sdk-nrfxlib/tree/main/nrf_fuel_gauge)
battery state-of-charge algorithm.

Scope of this crate, intentionally narrow:

- **Target:** nRF52840 (Cortex-M4, hard-float) — `thumbv7em-none-eabihf` only.
- **Variant:** secondary cell (rechargeable) only — links `libnrf_fuel_gauge.a`, not the
  `_primary` (non-rechargeable) variant.
- **Battery model:** a single model, fixed at build time. No bundled models, no runtime
  switching.

## Workspace layout

- `nrf-fuel-gauge-sys` — raw FFI bindings + build.rs linking logic.
  - `third_party/nordic/nrfxlib` — vendored copy of Nordic's header, license, and the one
    prebuilt `.a` this crate links. Committed to the repo, not fetched at build time.
- `nrf-fuel-gauge` — safe wrapper (`FuelGauge`).

## Vendored Nordic files

Following the same pattern as [nrf-sdc](https://github.com/alexmoon/nrf-sdc), Nordic's files are
vendored directly into `nrf-fuel-gauge-sys/third_party` and committed to this repo. They're pulled
from pinned release tags of <https://github.com/nrfconnect/sdk-nrfxlib> and
<https://github.com/nrfconnect/sdk-nrf> using `update-third-party.sh`

- `nrf_fuel_gauge.h`, `libnrf_fuel_gauge.a`, and Nordic's `license.txt`.
- `battery_model.inc` — the generic "Example" secondary-cell model bundled with sdk-nrf's
  [npm13xx_fuel_gauge sample](https://github.com/nrfconnect/sdk-nrf/tree/v3.4.0/samples/pmic/native/npm13xx_fuel_gauge),
  used as the default battery model until you supply your own (see below) — from `sdk-nrf`.

To refresh the vendored files:

```sh
./update-third-party.sh
```

## Battery model

The library needs a battery model tailored to your actual cell. Nordic doesn't ship
rechargeable-cell models for that in the SDK — generate one yourself with the **nPM PowerUp App**
(via an nPM1300 Evaluation Kit), which produces a `.inc` file in the same designated-initializer
style Nordic uses internally, targeting `struct battery_model`. Point the build at it:

```sh
NRF_FUEL_GAUGE_MODEL_PATH=/path/to/my_battery.inc cargo build
```

Without `NRF_FUEL_GAUGE_MODEL_PATH`, the build falls back to the vendored "Example" model
mentioned above — Nordic's own generic bring-up/test data for the nPM1300 fuel gauge sample. It's
a real, non-degenerate model (so you get plausible-looking SOC/TTE/TTF output to exercise the
runtime with), but it is **not calibrated for your battery** and must not be used in a real
product — swap in a model from the nPM PowerUp App before shipping. Unlike the Nordic SDK files
above, your real battery model is specific to your product and is not something this repo can
vendor for you — it stays external, referenced only by this env var.

## Prerequisites

- `rustup target add thumbv7em-none-eabihf`
- A cross C compiler for that target on `PATH`, e.g. `arm-none-eabi-gcc` — used only to compile
  the small generated shim that turns your `.inc` model file into a linkable symbol.

## Usage

```rust
use nrf_fuel_gauge::FuelGauge;

let (mut gauge, _adjusted_v0) = FuelGauge::init(3.7, 0.0, 22.0, None)?;
let soc = gauge.process(3.65, -0.05, 23.0, 1.0)?;
```

Only one `FuelGauge` can be alive at a time — the underlying C library holds a single global
instance, not a per-object one. `FuelGauge::init`/`init_from_state` return an `EBUSY` `Error` if
one already exists.

## Not covered yet

- Other Cortex-M variants / ESP32 / soft-float ABIs (Nordic ships prebuilt `.a` files for these
  too — extending `build.rs`'s target matching would be the way in if ever needed).
- Primary-cell (non-rechargeable) support.
- The optional `lock_func` synchronization callback (not exposed by the safe wrapper).
- Licensing review: vendoring `libnrf_fuel_gauge.a` means *this repo* now redistributes a binary
  under Nordic's `LicenseRef-Nordic-5-Clause`.
