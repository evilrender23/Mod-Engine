// Loose top-level code runs once, when the game starts.
global.score = 0
global.coins = 0
global.lives = 3
global.won = false

// Functions declared at the top level of any .ms file can be called from anywhere.
// They run "as" the caller: `self` is whoever called them.
function reset_run() {
    global.score = 0
    global.coins = 0
    global.lives = 3
    global.won = false
}

function burst(px, py, amount, tint = #ffcd75) {
    repeat amount {
        spawn(Spark, px, py, -20, { tint: tint })
    }
}

function jump_held() {
    return key_down(key.space) or key_down(key.up) or key_down(key.w) or key_down(key.z)
}

function jump_pressed() {
    return key_pressed(key.space) or key_pressed(key.up) or key_pressed(key.w) or key_pressed(key.z)
}

// Text with a dark drop shadow so it stays readable over any background.
function shadow_text(px, py, text, scale = 1, tint = #f4f4f4) {
    draw_text_ext(px + 1, py + 1, text, scale, #1a1c2c)
    draw_text_ext(px, py, text, scale, tint)
}
