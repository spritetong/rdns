// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
// rdns is licensed under the GNU GPL v3.0 or later.

//! Configuration module.

mod model;
mod parser;
mod validator;

pub use model::*;
pub use parser::load_config;
pub use validator::validate_config;
