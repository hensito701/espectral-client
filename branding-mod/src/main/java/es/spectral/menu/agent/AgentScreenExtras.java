package es.spectral.menu.agent;

import java.util.List;

/**
 * Implemented by Espectral screens that draw their own controls instead of
 * adding vanilla widgets (the main menu is one big custom canvas). The agent
 * {@code screen_tree} tool appends {@link #agentWidgets()} to the widget tree
 * so an agent can see — and click — the labelled regions.
 */
public interface AgentScreenExtras {

    /** Clickable regions in GUI-scaled coordinates, in layout order. */
    List<AgentWidget> agentWidgets();
}
