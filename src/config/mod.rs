// Copyright (c) 2026 Sprite Tong <spritetong@gmail.com>
// SPDX-License-Identifier: MPL-2.0
// rdns is licensed under the Mozilla Public License Version 2.0.

//! Configuration module.

mod model;
mod parser;
mod validator;

pub use model::*;
pub use parser::load_config;
pub use validator::validate_config;
