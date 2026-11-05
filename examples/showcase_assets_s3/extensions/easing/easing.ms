// Easing curves: t goes from 0 to 1.
function ease_out_back(t) {
    var c1 = 1.70158
    var c3 = c1 + 1
    return 1 + c3 * pow(t - 1, 3) + c1 * pow(t - 1, 2)
}

function ease_in_out_sine(t) {
    return -(cos(pi * t) - 1) / 2
}

function ease_out_bounce(t) {
    var n1 = 7.5625
    var d1 = 2.75
    if t < 1 / d1 return n1 * t * t
    if t < 2 / d1 {
        t -= 1.5 / d1
        return n1 * t * t + 0.75
    }
    if t < 2.5 / d1 {
        t -= 2.25 / d1
        return n1 * t * t + 0.9375
    }
    t -= 2.625 / d1
    return n1 * t * t + 0.984375
}
