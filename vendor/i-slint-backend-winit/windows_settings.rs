// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// cSpell: ignore GETCLIENTAREAANIMATION SETTINGCHANGE

//! Windows settings that the backend mirrors into the `SlintContext`.

use i_slint_core::MotionPreference;

/// featherplay patch (change 6): no preference, whatever the "Animation effects" switch under
/// Settings > Accessibility > Visual effects says. Upstream read it here
/// (`SPI_GETCLIENTAREAANIMATION`) at startup and on every focus gain, and on `Reduced` Slint
/// turns off every `animate` and makes a `Flickable`'s wheel notches jump. featherplay reads
/// the switch itself (`Theme.motion`, set from `apps/featherplay/src/app.rs`) and keeps its
/// fades, shorter, without motion (`ui/theme.slint`); its wheel filter already sends notches
/// straight through then (`src/wheel.rs`).
pub fn motion_preference() -> MotionPreference {
    MotionPreference::NoPreference
}
