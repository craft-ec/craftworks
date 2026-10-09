//! ONE LIST PER VOCABULARY (sdk#523, moved to layer 0 by sdk#524 so the engine's words use it too).

/// ONE LIST PER VOCABULARY, BY CONSTRUCTION (the architect on #523): the enum, its `code()` and its `ALL` are all
/// declared from ONE `Variant => "word"` list, so a variant cannot be left out of `ALL` (or `from_code`) -- there is
/// no second list to forget. Attributes (docs, `#[default]`, derives) pass through.
#[macro_export]
macro_rules! vocabulary {
    ($(#[$m:meta])* $vis:vis enum $name:ident { $($(#[$vm:meta])* $v:ident => $code:literal),+ $(,)? }) => {
        $(#[$m])*
        $vis enum $name {
            $($(#[$vm])* $v),+
        }

        impl $name {
            /// Every word, in declaration order: the same token list as the enum.
            pub const ALL: [$name; [$(stringify!($v)),+].len()] = [$($name::$v),+];

            /// The stable code that crosses the boundary.
            pub fn code(self) -> &'static str {
                match self {
                    $($name::$v => $code),+
                }
            }

            /// The word a code names; `None` for a code this build does not know.
            pub fn from_code(code: &str) -> Option<$name> {
                $name::ALL.into_iter().find(|w| w.code() == code)
            }
        }
    };
}
