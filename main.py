"""
TTT2 - Zen Browser First Launch Experience
Recreates the Zen Browser onboarding: transparent glass window over desktop wallpaper.
Uses tkinter + WebView2 via pywebview for proper Windows transparency.
"""

import webview
import os
import sys
import subprocess
import ctypes
import threading

# Enable DPI awareness for sharp rendering
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except:
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except:
        pass

def get_html_path():
    base_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(base_dir, "onboarding.html")

def apply_acrylic(hwnd):
    """
    Apply Windows Acrylic/Blur-Behind effect to a HWND.
    This gives the real frosted glass look like Zen Browser.
    """
    try:
        # Windows 10/11 accent policy for acrylic blur
        ACCENT_ENABLE_ACRYLICBLURBEHIND = 4
        ACCENT_ENABLE_BLURBEHIND = 3

        class ACCENT_POLICY(ctypes.Structure):
            _fields_ = [
                ("AccentState",   ctypes.c_uint),
                ("AccentFlags",   ctypes.c_uint),
                ("GradientColor", ctypes.c_uint),  # AABBGGRR
                ("AnimationId",   ctypes.c_uint),
            ]

        class WINDOWCOMPOSITIONATTRIBDATA(ctypes.Structure):
            _fields_ = [
                ("Attribute",  ctypes.c_int),
                ("Data",       ctypes.c_void_p),
                ("SizeOfData", ctypes.c_size_t),
            ]

        accent = ACCENT_POLICY()
        # Try acrylic first (Win10 1803+)
        accent.AccentState = ACCENT_ENABLE_ACRYLICBLURBEHIND
        # Dark tint: alpha=180 (0xB4), very dark navy (0x0f0a1e) → AABBGGRR = 0xB41e0a0f
        accent.GradientColor = 0xC0100820
        accent.AccentFlags = 0x20

        data = WINDOWCOMPOSITIONATTRIBDATA()
        data.Attribute = 19  # WCA_ACCENT_POLICY
        data.Data = ctypes.cast(ctypes.pointer(accent), ctypes.c_void_p)
        data.SizeOfData = ctypes.sizeof(accent)

        ctypes.windll.user32.SetWindowCompositionAttribute(hwnd, ctypes.pointer(data))
        print(f"[TTT2] Acrylic effect applied to HWND {hwnd}")
    except Exception as e:
        print(f"[TTT2] Acrylic fallback: {e}")

def on_loaded(window):
    """Called when the webview finishes loading — find the HWND and apply blur."""
    import time
    time.sleep(0.3)  # let the window settle
    try:
        hwnd = ctypes.windll.user32.FindWindowW(None, "Zen Browser")
        if hwnd:
            apply_acrylic(hwnd)
    except Exception as e:
        print(f"[TTT2] HWND lookup failed: {e}")

def main():
    html_path = get_html_path()

    if not os.path.exists(html_path):
        print(f"Error: Could not find {html_path}")
        sys.exit(1)

    window = webview.create_window(
        title="Zen Browser",
        url=html_path,
        width=960,
        height=640,
        resizable=False,
        frameless=True,
        easy_drag=True,
        background_color="#0f0820",
        text_select=False,
    )

    window.events.loaded += lambda: threading.Thread(
        target=on_loaded, args=(window,), daemon=True
    ).start()

    webview.start(debug=False)

if __name__ == "__main__":
    main()
