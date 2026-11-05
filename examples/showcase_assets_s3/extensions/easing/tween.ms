// Runs before the project's own scripts (extensions load first).
global.easing_ready = false

function tween(from, to, t, curve = ease_out_back) {
    return from + (to - from) * curve(clamp(t, 0, 1))
}

function easing_init() {
    global.easing_ready = true
}

function easing_final() {
    print("easing: adiós")
}
