//! Safe wrapper around Nordic Semiconductor's nRF Fuel Gauge library (secondary-cell /
//! rechargeable variant).
//!
//! The underlying C library holds a single hidden global instance — there is no "handle"
//! passed to its functions. [`FuelGauge`] models that as a guarded singleton: only one can be
//! alive at a time, enforced at runtime, and it is not [`Send`]/[`Sync`] since the C library
//! provides no internal synchronization unless you configure a lock function yourself.
//!
//! The battery model is fixed at build time (see the `nrf-fuel-gauge-sys` crate's
//! `NRF_FUEL_GAUGE_MODEL_PATH`) rather than chosen at runtime — this wrapper does not support
//! switching battery models.

#![no_std]

use core::ffi::c_void;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};

use nrf_fuel_gauge_sys as sys;

pub use sys::nrf_fuel_gauge_state_info as StateInfo;

/// Configuration parameters. Obtain a starting point with [`config_params_default`] or
/// [`FuelGauge::config_params_current`], then adjust individual fields.
pub type ConfigParameters = sys::nrf_fuel_gauge_config_parameters;

const EBUSY: i32 = -16;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// An error returned by the underlying nRF Fuel Gauge library, wrapping its raw (negative)
/// error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(i32);

impl Error {
    fn check(ret: i32) -> Result<(), Self> {
        if ret == 0 {
            Ok(())
        } else {
            Err(Error(ret))
        }
    }

    /// The raw negative error code returned by the C library.
    pub fn raw(&self) -> i32 {
        self.0
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "nrf_fuel_gauge error (errno {})", -self.0)
    }
}

/// Charger state, as reported by the charger device — see [`ExtStateInfo::ChargeStateChange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeState {
    Idle,
    Trickle,
    Cc,
    CcLimited,
    Cv,
    Complete,
}

impl ChargeState {
    fn to_ffi(self) -> sys::nrf_fuel_gauge_charge_state {
        use sys::nrf_fuel_gauge_charge_state as Ffi;
        match self {
            ChargeState::Idle => Ffi::Idle,
            ChargeState::Trickle => Ffi::Trickle,
            ChargeState::Cc => Ffi::Cc,
            ChargeState::CcLimited => Ffi::CcLimited,
            ChargeState::Cv => Ffi::Cv,
            ChargeState::Complete => Ffi::Complete,
        }
    }
}

/// External event/factor to report via [`FuelGauge::ext_state_update`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtStateInfo {
    VbusConnected,
    VbusDisconnected,
    BatteryReplaced,
    ChargeStateChange(ChargeState),
    /// Multiply current by this factor when charging.
    ChargeCurrentCorrection(f32),
    /// Multiply current by this factor when discharging.
    DischargeCurrentCorrection(f32),
    /// Charge current limit used by the charger, in amperes.
    ChargeCurrentLimit(f32),
    /// Charge termination current used by the charger, in amperes.
    TermCurrent(f32),
}

/// Handle to the (singleton) nRF Fuel Gauge library instance.
///
/// Only one instance may exist at a time; [`FuelGauge::init`] / [`FuelGauge::init_from_state`]
/// return [`Error`] (`EBUSY`) if one is already alive. Dropping it frees the slot for a new
/// instance, but does **not** reset the underlying library's internal filter state — there is
/// no such operation in the upstream API.
pub struct FuelGauge {
    // Not Send/Sync: the C library's global state has no synchronization unless a lock_func is
    // configured (not currently exposed by this wrapper).
    _not_send_sync: PhantomData<*const ()>,
}

impl FuelGauge {
    /// Initializes the library from initial measurements.
    ///
    /// Returns the handle along with the adjusted initial voltage (for logging purposes).
    pub fn init(
        v0: f32,
        i0: f32,
        t0: f32,
        config: Option<&ConfigParameters>,
    ) -> Result<(Self, f32), Error> {
        let opt_params = config.map_or(core::ptr::null(), |c| c as *const _);
        Self::init_raw(v0, i0, t0, opt_params, core::ptr::null(), 0)
    }

    /// Resumes from a previously stored state (see [`FuelGauge::state`]).
    ///
    /// Per upstream semantics, `config` is ignored when resuming from a state — only the
    /// initial measurements and the stored state matter.
    pub fn init_from_state(v0: f32, i0: f32, t0: f32, state: &[u8]) -> Result<(Self, f32), Error> {
        Self::init_raw(
            v0,
            i0,
            t0,
            core::ptr::null(),
            state.as_ptr().cast::<c_void>(),
            state.len(),
        )
    }

    fn init_raw(
        v0: f32,
        i0: f32,
        t0: f32,
        opt_params: *const ConfigParameters,
        state: *const c_void,
        state_size: usize,
    ) -> Result<(Self, f32), Error> {
        if INITIALIZED.swap(true, Ordering::AcqRel) {
            return Err(Error(EBUSY));
        }

        let params = sys::nrf_fuel_gauge_init_parameters {
            v0,
            i0,
            t0,
            model: core::ptr::addr_of!(sys::nrf_fuel_gauge_wrapper_model),
            opt_params,
            lock_func: None,
            lock_context: core::ptr::null(),
            state,
            state_size,
        };

        let mut adjusted_v0 = 0.0f32;
        // Safety: `params` is a valid, fully-initialized struct; `adjusted_v0` is a valid
        // out-pointer. The model pointer stays valid for the library's lifetime since it points
        // at a `'static` symbol compiled in by build.rs.
        let ret = unsafe { sys::nrf_fuel_gauge_init(&params, &mut adjusted_v0) };

        if let Err(e) = Error::check(ret) {
            INITIALIZED.store(false, Ordering::Release);
            return Err(e);
        }

        Ok((
            Self {
                _not_send_sync: PhantomData,
            },
            adjusted_v0,
        ))
    }

    /// Processes a new battery measurement, returning the predicted state-of-charge [%].
    pub fn process(&mut self, v: f32, i: f32, t: f32, t_delta: f32) -> Result<f32, Error> {
        let mut soc = 0.0f32;
        let ret = unsafe {
            sys::nrf_fuel_gauge_process(v, i, t, t_delta, &mut soc, core::ptr::null_mut())
        };
        Error::check(ret)?;
        Ok(soc)
    }

    /// Like [`FuelGauge::process`], but also returns the internal debug state info.
    pub fn process_with_state(
        &mut self,
        v: f32,
        i: f32,
        t: f32,
        t_delta: f32,
    ) -> Result<(f32, StateInfo), Error> {
        let mut soc = 0.0f32;
        let mut state = StateInfo::default();
        let ret = unsafe { sys::nrf_fuel_gauge_process(v, i, t, t_delta, &mut soc, &mut state) };
        Error::check(ret)?;
        Ok((soc, state))
    }

    /// Most recently calculated state-of-charge [%].
    pub fn soc(&self) -> Result<f32, Error> {
        let mut soc = 0.0f32;
        Error::check(unsafe { sys::nrf_fuel_gauge_soc_get(&mut soc) })?;
        Ok(soc)
    }

    /// Predicted time-to-empty [s]. May be `NaN` if not yet available.
    pub fn tte(&self) -> Result<f32, Error> {
        let mut tte = 0.0f32;
        Error::check(unsafe { sys::nrf_fuel_gauge_tte_get(&mut tte) })?;
        Ok(tte)
    }

    /// Predicted time-to-full [s]. May be `NaN` if not yet available.
    pub fn ttf(&self) -> Result<f32, Error> {
        let mut ttf = 0.0f32;
        Error::check(unsafe { sys::nrf_fuel_gauge_ttf_get(&mut ttf) })?;
        Ok(ttf)
    }

    /// Predicted state-of-health [%].
    pub fn soh(&self) -> Result<f32, Error> {
        let mut soh = 0.0f32;
        Error::check(unsafe { sys::nrf_fuel_gauge_soh_get(&mut soh) })?;
        Ok(soh)
    }

    /// Informs the library of an expected average current during an idle/low-power period.
    pub fn idle_set(&mut self, v: f32, t: f32, i_avg: f32) -> Result<(), Error> {
        Error::check(unsafe { sys::nrf_fuel_gauge_idle_set(v, t, i_avg) })
    }

    /// Informs the library of an external event or factor (charger state, VBUS, etc.).
    pub fn ext_state_update(&mut self, info: ExtStateInfo) -> Result<(), Error> {
        use sys::{nrf_fuel_gauge_ext_state_info_data as Data, nrf_fuel_gauge_ext_state_info_type as Ty};

        let (info_type, mut data) = match info {
            ExtStateInfo::VbusConnected => (Ty::VbusConnected, None),
            ExtStateInfo::VbusDisconnected => (Ty::VbusDisconnected, None),
            ExtStateInfo::BatteryReplaced => (Ty::BatteryReplaced, None),
            ExtStateInfo::ChargeStateChange(cs) => (
                Ty::ChargeStateChange,
                Some(Data {
                    charge_state: cs.to_ffi(),
                }),
            ),
            ExtStateInfo::ChargeCurrentCorrection(f) => (
                Ty::ChargeCurrentCorrection,
                Some(Data {
                    current_correction_factor: f,
                }),
            ),
            ExtStateInfo::DischargeCurrentCorrection(f) => (
                Ty::DischargeCurrentCorrection,
                Some(Data {
                    current_correction_factor: f,
                }),
            ),
            ExtStateInfo::ChargeCurrentLimit(f) => (
                Ty::ChargeCurrentLimit,
                Some(Data {
                    charge_current_limit: f,
                }),
            ),
            ExtStateInfo::TermCurrent(f) => (
                Ty::TermCurrent,
                Some(Data {
                    charge_term_current: f,
                }),
            ),
        };

        let data_ptr = data
            .as_mut()
            .map_or(core::ptr::null_mut(), |d| d as *mut _);
        Error::check(unsafe { sys::nrf_fuel_gauge_ext_state_update(info_type, data_ptr) })
    }

    /// Currently active configuration parameters.
    pub fn config_params_current(&self) -> Result<ConfigParameters, Error> {
        let mut params = ConfigParameters::default();
        Error::check(unsafe { sys::nrf_fuel_gauge_config_params_current_get(&mut params) })?;
        Ok(params)
    }

    /// Updates configuration parameters. Fields set to `NaN` are left unchanged.
    pub fn config_params_adjust(&mut self, params: &ConfigParameters) -> Result<(), Error> {
        Error::check(unsafe { sys::nrf_fuel_gauge_config_params_adjust(params) })
    }

    /// Size in bytes of the buffer required by [`FuelGauge::state`].
    pub fn state_len(&self) -> usize {
        unsafe { sys::nrf_fuel_gauge_state_size }
    }

    /// Copies the library's internal state into `buf`, for persisting across a period without
    /// RAM retention. `buf` must be at least [`FuelGauge::state_len`] bytes.
    pub fn state(&self, buf: &mut [u8]) -> Result<(), Error> {
        Error::check(unsafe {
            sys::nrf_fuel_gauge_state_get(buf.as_mut_ptr().cast::<c_void>(), buf.len())
        })
    }
}

impl Drop for FuelGauge {
    fn drop(&mut self) {
        INITIALIZED.store(false, Ordering::Release);
    }
}

/// Default configuration parameter values.
pub fn config_params_default() -> Result<ConfigParameters, Error> {
    let mut params = ConfigParameters::default();
    Error::check(unsafe { sys::nrf_fuel_gauge_config_params_default_get(&mut params) })?;
    Ok(params)
}

/// Checks whether a previously stored state (see [`FuelGauge::state`]) is compatible with the
/// currently linked library version.
pub fn state_compatible(state: &[u8]) -> bool {
    unsafe {
        sys::nrf_fuel_gauge_state_compatible_check(state.as_ptr().cast::<c_void>(), state.len())
            == 0
    }
}
