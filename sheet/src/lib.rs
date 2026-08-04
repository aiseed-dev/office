//! sheet — xlsx の読み書きと表計算の核。UI非依存。
//!
//! **マクロは実装しない。** 機能不足ではなく設計判断:
//! 文書の中に特権実行コードが同居する形(VBA)をやめ、
//! 「開く=実行」という攻撃経路を最初から持たない
//! (aiseed-migration-kit DESIGN.md §5 と同じ思想)。

pub mod calc;
pub mod model;
pub mod styles;
pub mod theme;
pub mod xlsx;

pub use calc::recalc;
pub use model::{Book, Cell, Pos, Sheet, Value};
