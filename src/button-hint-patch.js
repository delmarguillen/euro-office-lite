    // Patch Button.updateHint to tolerate undefined hints (tipSmartPicker etc.)
    // Uses polling because the iframe loads modules asynchronously via require.js
    window._patchButtonHint = function(win) {
      try {
        var checkBtn = setInterval(function() {
          try {
            if (win.Common && win.Common.UI && win.Common.UI.Button) {
              var proto = win.Common.UI.Button.prototype;
              var origUpdateHint = proto.updateHint;
              proto.updateHint = function(hint, isHtml) {
                if (hint === undefined || hint === null) return;
                return origUpdateHint.call(this, hint, isHtml);
              };
              clearInterval(checkBtn);
            }
          } catch(e) { clearInterval(checkBtn); }
        }, 10);
        setTimeout(function() { clearInterval(checkBtn); }, 30000);
      } catch(e) {}
    };
