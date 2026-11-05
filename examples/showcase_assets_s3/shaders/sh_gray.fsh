// Grayscale; `u_strength` 0 = colour, 1 = gray. (No .vsh: the default one is used.)
varying vec2 v_vTexcoord;
varying vec4 v_vColour;

uniform float u_strength;

void main()
{
    vec4 c = v_vColour * texture2D(gm_BaseTexture, v_vTexcoord);
    float g = dot(c.rgb, vec3(0.299, 0.587, 0.114));
    gl_FragColor = vec4(mix(c.rgb, vec3(g), u_strength), c.a);
}
