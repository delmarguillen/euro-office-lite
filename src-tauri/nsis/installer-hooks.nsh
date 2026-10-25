; DoctRenderer writes a V8 code cache next to each sdkjs bundle the first time it
; runs, and only when the file is missing. Once an update ships other bundles, V8
; rejects the old cache and every PDF export compiles them from scratch, so drop
; the caches here and let the next export write fresh ones.
; Earlier installers also shipped binaries\fontdata, a font cache built on another
; machine that the app never uses (it builds its own in the temp folder), and
; converter files nothing reads: an old DoctRenderer setup, copies of the sdkjs
; bundles and two extra copies of the bundled fonts. They also ran x2t from the
; install folder, where the app wrote a DoctRenderer.config; x2t now runs from
; binaries\. Updating does not remove files the new version no longer ships.
; Exact names only.

; The 16 bundled faces in one folder. binaries\fonts keeps its own copies.
!macro EOL_DELETE_BUNDLED_FACES DIR
  Delete "${DIR}\Carlito-Bold.ttf"
  Delete "${DIR}\Carlito-BoldItalic.ttf"
  Delete "${DIR}\Carlito-Italic.ttf"
  Delete "${DIR}\Carlito-Regular.ttf"
  Delete "${DIR}\LiberationMono-Bold.ttf"
  Delete "${DIR}\LiberationMono-BoldItalic.ttf"
  Delete "${DIR}\LiberationMono-Italic.ttf"
  Delete "${DIR}\LiberationMono-Regular.ttf"
  Delete "${DIR}\LiberationSans-Bold.ttf"
  Delete "${DIR}\LiberationSans-BoldItalic.ttf"
  Delete "${DIR}\LiberationSans-Italic.ttf"
  Delete "${DIR}\LiberationSans-Regular.ttf"
  Delete "${DIR}\LiberationSerif-Bold.ttf"
  Delete "${DIR}\LiberationSerif-BoldItalic.ttf"
  Delete "${DIR}\LiberationSerif-Italic.ttf"
  Delete "${DIR}\LiberationSerif-Regular.ttf"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\editors\sdkjs\word\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\cell\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\slide\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\pdf\sdk-all.cache"
  Delete "$INSTDIR\editors\sdkjs\visio\sdk-all.cache"
  Delete "$INSTDIR\binaries\fontdata\AllFonts.js"
  Delete "$INSTDIR\binaries\fontdata\font_selection.bin"
  Delete "$INSTDIR\binaries\fontdata\fonts.log"
  RMDir "$INSTDIR\binaries\fontdata"
  Delete "$INSTDIR\binaries\sdk-word-bundle.js"
  Delete "$INSTDIR\binaries\native_shim.js"
  Delete "$INSTDIR\binaries\DoctRenderer.config"
  Delete "$INSTDIR\binaries\xregexp-all-min.js"
  Delete "$INSTDIR\binaries\sdk-all.js"
  Delete "$INSTDIR\binaries\sdk-all-min.js"
  Delete "$INSTDIR\binaries\package.config"
  !insertmacro EOL_DELETE_BUNDLED_FACES "$INSTDIR\binaries"
  !insertmacro EOL_DELETE_BUNDLED_FACES "$INSTDIR\binaries\core-fonts"
  RMDir "$INSTDIR\binaries\core-fonts"
  Delete "$INSTDIR\x2t.exe"
  Delete "$INSTDIR\DoctRenderer.config"
  ; A file that could not be deleted sets the error flag, and .onInstSuccess reads
  ; it for the /R restart option.
  ClearErrors
!macroend
