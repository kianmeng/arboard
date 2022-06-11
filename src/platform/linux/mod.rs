#[cfg(feature = "image-data")]
use std::{cell::RefCell, rc::Rc};

#[cfg(feature = "wayland-data-control")]
use log::{info, warn};

use crate::Error;
#[cfg(feature = "image-data")]
use crate::ImageData;

mod x11;

#[cfg(feature = "wayland-data-control")]
mod wayland;

fn into_unknown<E: std::fmt::Display>(error: E) -> Error {
	Error::Unknown { description: format!("{}", error) }
}

#[cfg(feature = "image-data")]
fn encode_as_png(image: &ImageData) -> Result<Vec<u8>, Error> {
	/// This is a workaround for the PNGEncoder not having a `into_inner` like function
	/// which would allow us to take back our Vec after the encoder finished encoding.
	/// So instead we create this wrapper around an Rc Vec which implements `io::Write`
	#[derive(Clone)]
	struct RcBuffer {
		inner: Rc<RefCell<Vec<u8>>>,
	}
	impl std::io::Write for RcBuffer {
		fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
			self.inner.borrow_mut().extend_from_slice(buf);
			Ok(buf.len())
		}
		fn flush(&mut self) -> std::io::Result<()> {
			// Noop
			Ok(())
		}
	}

	if image.bytes.is_empty() || image.width == 0 || image.height == 0 {
		return Err(Error::ConversionFailure);
	}

	let enc_output = RcBuffer { inner: Rc::new(RefCell::new(Vec::new())) };
	let encoder = image::png::PngEncoder::new(enc_output.clone());
	encoder
		.encode(
			image.bytes.as_ref(),
			image.width as u32,
			image.height as u32,
			image::ColorType::Rgba8,
		)
		.map_err(|_| Error::ConversionFailure)?;

	// The encoder must be destroyed by the time we get to `try_unwrap`, in order to
	// be able to take the value from the `Rc`. This code is currently relying on the fact
	// that the `encode` function consumes its `self` parameter.
	let bytes = Rc::try_unwrap(enc_output.inner).unwrap().into_inner();
	Ok(bytes)
}

/// Clipboard selection
///
/// Linux has a concept of clipboard "selections" which tend to be used in different contexts. This
/// enum provides a way to get/set to a specific clipboard (the default
/// [`Clipboard`](Self::Clipboard) being used for the common platform API). You can choose which
/// clipboard to use with [`GetExtLinux::clipboard`] and [`SetExtLinux::clipboard`].
///
/// See <https://specifications.freedesktop.org/clipboards-spec/clipboards-0.1.txt> for a better
/// description of the different clipboards.
#[derive(Copy, Clone, Debug)]
pub enum LinuxClipboardKind {
	/// Typically used selection for explicit cut/copy/paste actions (ie. windows/macos like
	/// clipboard behavior)
	Clipboard,

	/// Typically used for mouse selections and/or currently selected text. Accessible via middle
	/// mouse click.
	///
	/// *On Wayland, this may not be available for all systems (requires a compositor supporting
	/// version 2 or above) and operations using this will return an error if unsupported.*
	Primary,

	/// The secondary clipboard is rarely used but theoretically available on X11.
	///
	/// *On Wayland, this is not be available and operations using this variant will return an
	/// error.*
	Secondary,
}

pub(crate) enum Clipboard {
	X11(x11::Clipboard),

	#[cfg(feature = "wayland-data-control")]
	WlDataControl(wayland::Clipboard),
}

impl Clipboard {
	pub(crate) fn new() -> Result<Self, Error> {
		#[cfg(feature = "wayland-data-control")]
		{
			if std::env::var_os("WAYLAND_DISPLAY").is_some() {
				// Wayland is available
				match wayland::Clipboard::new() {
					Ok(clipboard) => {
						info!("Successfully initialized the Wayland data control clipboard.");
						return Ok( Self::WlDataControl(clipboard))
					},
					Err(e) => warn!(
						"Tried to initialize the wayland data control protocol clipboard, but failed. Falling back to the X11 clipboard protocol. The error was: {}",
						e
					),
				}
			}
		}
		Ok(Self::X11(x11::Clipboard::new()?))
	}
}

pub(crate) struct Get<'clipboard> {
	clipboard: &'clipboard mut Clipboard,
	selection: LinuxClipboardKind,
}

impl<'clipboard> Get<'clipboard> {
	pub(crate) fn new(clipboard: &'clipboard mut Clipboard) -> Self {
		Self { clipboard, selection: LinuxClipboardKind::Clipboard }
	}

	pub(crate) fn text(self) -> Result<String, Error> {
		match self.clipboard {
			Clipboard::X11(clipboard) => clipboard.get_text(self.selection),
			#[cfg(feature = "wayland-data-control")]
			Clipboard::WlDataControl(clipboard) => clipboard.get_text(self.selection),
		}
	}

	#[cfg(feature = "image-data")]
	pub(crate) fn image(self) -> Result<ImageData<'static>, Error> {
		match self.clipboard {
			Clipboard::X11(clipboard) => clipboard.get_image(self.selection),
			#[cfg(feature = "wayland-data-control")]
			Clipboard::WlDataControl(clipboard) => clipboard.get_image(self.selection),
		}
	}
}

/// Linux-specific extensions to the [`Get`](super::Get) builder.
pub trait GetExtLinux {
	/// Sets the clipboard the operation will retrieve data from.
	///
	/// If wayland support is enabled and available, attempting to use the Secondary clipboard will
	/// return an error.
	fn clipboard(self, selection: LinuxClipboardKind) -> Self;
}

impl GetExtLinux for crate::Get<'_> {
	fn clipboard(mut self, selection: LinuxClipboardKind) -> Self {
		self.platform.selection = selection;
		self
	}
}

pub(crate) struct Set<'clipboard> {
	clipboard: &'clipboard mut Clipboard,
	wait: bool,
	selection: LinuxClipboardKind,
}

impl<'clipboard> Set<'clipboard> {
	pub(crate) fn new(clipboard: &'clipboard mut Clipboard) -> Self {
		Self { clipboard, wait: false, selection: LinuxClipboardKind::Clipboard }
	}

	pub(crate) fn text(self, text: String) -> Result<(), Error> {
		match self.clipboard {
			Clipboard::X11(clipboard) => clipboard.set_text(text, self.selection, self.wait),
			#[cfg(feature = "wayland-data-control")]
			Clipboard::WlDataControl(clipboard) => clipboard.set_text(text, self.selection, self.wait),
		}
	}

	#[cfg(feature = "image-data")]
	pub(crate) fn image(self, image: ImageData<'_>) -> Result<(), Error> {
		match self.clipboard {
			Clipboard::X11(clipboard) => clipboard.set_image(image, self.selection, self.wait),
			#[cfg(feature = "wayland-data-control")]
			Clipboard::WlDataControl(clipboard) => clipboard.set_image(image, self.selection, self.wait),
		}
	}
}

/// Linux specific extensions to the [`Set`](super::Set) builder.
pub trait SetExtLinux {
	/// Whether to wait for the clipboard's contents to be replaced after setting it.
	///
	/// The Wayland and X11 clipboards work by having the clipboard content being, at any given
	/// time, "owned" by a single process, and that process is expected to reply to all the requests
	/// from any other system process that wishes to access the clipboard's contents. As a
	/// consequence, when that process exits the contents of the clipboard will effectively be
	/// cleared since there is no longer anyone around to serve requests for it.
	///
	/// This poses a problem for short-lived programs that just want to copy to the clipboard and
	/// then exit, since they don't want to wait until the user happens to copy something else just
	/// to finish. To resolve that, whenever the user copies something you can offload the actual
	/// work to a newly-spawned daemon process which will run in the background (potentially
	/// outliving the current process) and serve all the requests. That process will then
	/// automatically and silently exit once the user copies something else to their clipboard so it
	/// doesn't take up too many resources.
	///
	/// To support that pattern, this method will not only have the contents of the clipboard be
	/// set, but will also wait and continue to serve requests until the clipboard is overwritten.
	/// As long as you don't exit the current process until that method has returned, you can avoid
	/// all surprising situations where the clipboard's contents seemingly disappear from under your
	/// feet.
	///
	/// See the [daemonize example] for a demo of how you could implement this.
	///
	/// [daemonize example]: https://github.com/1Password/arboard/blob/master/examples/daemonize.rs
	fn wait(self) -> Self;

	/// Sets the clipboard the operation will store its data to.
	///
	/// If wayland support is enabled and available, attempting to use the Secondary clipboard will
	/// return an error.
	///
	/// # Examples
	///
	/// ```
	/// use arboard::{Clipboard, SetExtLinux, LinuxClipboardKind};
	/// # fn main() -> Result<(), arboard::Error> {
	/// let mut ctx = Clipboard::new()?;
	///
	/// let clipboard = "This goes in the traditional (ex. Copy & Paste) clipboard.";
	/// ctx.set().clipboard(LinuxClipboardKind::Clipboard).text(clipboard.to_owned())?;
	///
	/// let primary = "This goes in the primary keyboard. It's typically used via middle mouse click.";
	/// ctx.set().clipboard(LinuxClipboardKind::Primary).text(primary.to_owned())?;
	/// # Ok(())
	/// # }
	/// ```
	fn clipboard(self, selection: LinuxClipboardKind) -> Self;
}

impl SetExtLinux for crate::Set<'_> {
	fn wait(mut self) -> Self {
		self.platform.wait = true;
		self
	}

	fn clipboard(mut self, selection: LinuxClipboardKind) -> Self {
		self.platform.selection = selection;
		self
	}
}