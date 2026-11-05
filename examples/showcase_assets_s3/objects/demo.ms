// Shows the three asset types of this example.
object Demo {
    fire = -1
    snow = -1
    trail = -1
    star = -1
    t = 0

    on create {
        // particle systems from assets
        fire = part_system_create(ps_fire)
        part_system_position(fire, 60, 150)
        snow = part_system_create(ps_snow)

        // a system built in code: a trail of stars that follows the mouse
        trail = part_system_create()
        part_system_depth(trail, -20)
        star = part_type_create()
        part_type_sprite(star, spark, false, false, false)
        part_type_size(star, 0.6, 1.2, -0.01, 0)
        part_type_speed(star, 0.2, 0.6, 0, 0)
        part_type_direction(star, 0, 360, 2, 0)
        part_type_orientation(star, 0, 360, 4, 0, false)
        part_type_color3(star, #ffcd75, #ef7d57, #b13e53)
        part_type_alpha2(star, 1, 0)
        part_type_life(star, 30, 50)
        part_type_blend(star, true)
        alarm[0] = 90
    }

    on step {
        t += 1
        part_particles_create(trail, 160 + dcos(t * 2) * 60, 90 - dsin(t * 3) * 30, star, 1)
    }

    on alarm(0) {
        part_particles_burst(trail, 250 + rand_range(0, 40) - 20, 120, ps_burst)
        alarm[0] = 90
    }

    on draw {
        // the torch under the fire
        draw_sprite(torch, 0, 60, 158)
        // a crystal drawn with the wave shader
        shader_set(sh_wave)
        shader_set_uniform_f(shader_get_uniform(sh_wave, "u_time"), time())
        shader_set_uniform_f(shader_get_uniform(sh_wave, "u_amount"), 1)
        draw_sprite_ext(crystal, 0, 160, 92, 3 * tween(0.5, 1, t / EASE_DEFAULT_STEPS), 3 * tween(0.5, 1, t / EASE_DEFAULT_STEPS), 0, #ffffff, 1)
        shader_reset()
        // the same crystal in grayscale, fading in and out
        shader_set(sh_gray)
        shader_set_uniform_f(shader_get_uniform(sh_gray, "u_strength"), ease_in_out_sine(0.5 + 0.5 * dsin(t * 2)))
        draw_sprite_ext(crystal, 0, 250, 60, 2, 2, t, #ffffff, 1)
        shader_reset()
    }

    on draw_gui {
        set_color(#f4f4f4)
        draw_text(4, 4, "partículas: " + str(part_particles_count(fire) + part_particles_count(snow) + part_particles_count(trail)))
        draw_text(4, 16, "easing " + str(EASING_VERSION) + (global.easing_ready ? " listo" : " sin init"))
        draw_text(4, 28, "shaders: " + (shader_is_compiled(sh_wave) ? "ok" : "ERROR"))
    }
}
