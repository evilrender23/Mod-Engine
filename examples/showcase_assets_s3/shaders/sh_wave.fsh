// Wavy distortion plus a colour pulse. `u_time` is set from ModScript every frame.
varying vec2 v_vTexcoord;
varying vec4 v_vColour;

uniform float u_time;
uniform float u_amount;

void main()
{
    vec2 uv = v_vTexcoord;
    uv.x += sin(uv.y * 40.0 + u_time * 4.0) * 0.03 * u_amount;
    vec4 c = texture2D(gm_BaseTexture, uv);
    float pulse = 0.5 + 0.5 * sin(u_time * 3.0);
    c.rgb += vec3(0.25, 0.1, 0.4) * pulse * c.a;
    gl_FragColor = v_vColour * c;
}
