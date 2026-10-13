; DoctRenderer writes a V8 code cache next to each sdkjs bundle the first time it
; runs, and only when the file is missing. Once an update ships other bundles, V8
; rejects the old cache and every PDF export compiles them from scratch, so drop
; the caches here and let the next export write fresh ones. Exact names only.
!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\editors\sdkjs\word\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\cell\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\slide\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\pdf\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\visio\sdk-all.cache"
  ; A file that could not be deleted sets the error flag, and .onInstSuccess reads
  ; it for the /R restart option.
  ClearErrors
!macroend
