// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 R5OS Contributors
//
// The C++ view is linked by scripts/build-r5servo.sh into libr5servo.so.
// A Rust cdylib hides those symbols, so the shared library is produced
// by the Haiku g++ from this staticlib plus BServoView.cpp.

fn main() {}
