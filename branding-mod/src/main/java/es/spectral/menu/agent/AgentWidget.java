package es.spectral.menu.agent;

/**
 * One clickable region a custom-drawn screen exposes to the agent
 * {@code screen_tree} tool. Vanilla screens are walked through the widget
 * tree; the Espectral menus paint their buttons themselves, so they implement
 * {@link AgentScreenExtras} and hand over the same information.
 *
 * <p>Ids are stable for as long as the screen's layout does not change, which
 * is what {@code click {id}} relies on.
 */
public record AgentWidget(String id, String type, String text,
        int x, int y, int w, int h) {
}
