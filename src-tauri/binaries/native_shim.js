// Stubs for DoctRenderer native context (x2t PDF conversion)
// browser.js and device_scale.js expect browser APIs that don't exist in V8

window.devicePixelRatio = 1;

AscCommon.checkDeviceScale = function() {
    return { zoom: 1, devicePixelRatio: 1, applicationPixelRatio: 1, correct: false };
};

AscCommon.correctApplicationScale = function() {};

AscCommon.AscBrowser = {
    isAndroidNativeApp: false,
    isIE: false,
    isArm: false,
    isChrome: true,
    isSafari: false,
    isMozilla: false,
    isOpera: false,
    isRetina: false,
    retinaPixelRatio: 1,
    isWebkit: true,
    isMobile: false,
    checkZoom: function() {}
};

// Prevent crash when AllFonts.js is not loaded
window["g_fonts_selection_bin"] = "";
window["__fonts_infos"] = [];
