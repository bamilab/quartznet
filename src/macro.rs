// Credits to Shepmaster on stackoverflow.com for this idea.
// It needed a small syntax correction though.
/// This macro allows you to create an enum which automatically implements TryFrom and Into for an integer type.
#[macro_export]
macro_rules! int_enum {
	($(#[$meta:meta])* $vis:vis enum $name:ident: $int:ty {
		$($(#[$vmeta:meta])* $vname:ident $(= $val:expr)?),*
	}) => {
		$(#[$meta])*
		$vis enum $name {
			$($(#[$vmeta])* $vname $(= $val)?,)*
		}

		impl std::convert::TryFrom<$int> for $name {
			type Error = ();

			fn try_from(v: $int) -> Result<Self, Self::Error> {
				match v {
					$(x if x == Self::$vname as $int => Ok(Self::$vname),)*
					_ => Err(()),
				}
			}
		}

		impl Into<$int> for $name {
			fn into( self ) -> $int {
				self as $int
			}
		}
	}
}

/// Same as `int_enum!`, but for `u8` only.
#[macro_export]
macro_rules! byte_enum {
	($(#[$meta:meta])* $vis:vis enum $name:ident {
		$($(#[$vmeta:meta])* $vname:ident $(= $val:expr)?),*
	}) => {
		crate::int_enum!($(#[$meta])* $vis enum $name: u8 {
			$($(#[$vmeta])* $vname $(= $val)?),*
		});
	}
}