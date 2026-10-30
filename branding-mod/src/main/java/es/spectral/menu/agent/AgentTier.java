package es.spectral.menu.agent;

/**
 * Permission tier of one agent tool, mirroring
 * {@code docs/agent-capture-plan.md} → Protocol v1. The mod is the enforcement
 * point (the engine only mirrors the answer for clearer errors).
 *
 * <ul>
 *   <li>{@link #OBSERVE} / {@link #CAPTURE} — passive, allowed everywhere an
 *       agent launch exists.</li>
 *   <li>{@link #CAMERA} — detached camera / cinematic renders: not on public
 *       servers (freecam there is seeing through walls).</li>
 *   <li>{@link #ACT} — input, GUI clicks, chat: singleplayer, replay viewer,
 *       localhost and the allowlist only; never on Espectral's own hosts.</li>
 * </ul>
 */
public enum AgentTier {
    OBSERVE,
    CAPTURE,
    CAMERA,
    ACT;

    /** True when {@code context} may run tools of this tier. */
    public boolean allows(AgentPolicy.Context context) {
        if (context == null) return false;
        if (this == OBSERVE || this == CAPTURE) return true;
        switch (context) {
            case MENU:
                // The Suite/title menus are client-side UI: the P2 acceptance
                // is an agent driving them with no world loaded at all.
                return true;
            case SINGLEPLAYER:
            case REPLAY:
            case LOCALHOST:
            case ALLOWLISTED:
                return true;
            default:
                // ESPECTRAL and SERVER: act/camera input is refused.
                return false;
        }
    }
}
