use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::models::*;
use crate::declare::PrintHtmlOptions;

pub fn init<R: Runtime, C: DeserializeOwned>(
  app: &AppHandle<R>,
  _api: PluginApi<R, C>,
) -> crate::Result<Printer<R>> {
  Ok(Printer(app.clone()))
}

/// Access to the printer APIs.
pub struct Printer<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Printer<R> {
  pub fn ping(&self, payload: PingRequest) -> crate::Result<PingResponse> {
    Ok(PingResponse {
      value: payload.value,
    })
  }

  pub fn get_printers(&self) -> crate::Result<String> {
    #[cfg(target_os = "linux")]
    {
      crate::linux::get_printers().map_err(crate::Error::Printing)
    }
    #[cfg(target_os = "windows")]
    {
      Ok(crate::windows::get_printers())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
      Err(crate::Error::UnsupportedPlatform)
    }
  }

  pub fn get_printer_by_name(&self, name: String) -> crate::Result<String> {
    #[cfg(target_os = "windows")]
    {
      Ok(crate::windows::get_printers_by_name(name))
    }
    #[cfg(not(target_os = "windows"))]
    {
      Err(crate::Error::UnsupportedPlatform)
    }
  }

  pub fn print_html(&self, options: PrintHtmlOptions) -> crate::Result<String> {
    #[cfg(target_os = "windows")]
    {
      Ok(crate::windows::print_html(options))
    }
    #[cfg(not(target_os = "windows"))]
    {
      Err(crate::Error::UnsupportedPlatform)
    }
  }
}
