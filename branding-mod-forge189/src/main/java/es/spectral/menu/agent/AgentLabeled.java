package es.spectral.menu.agent;

import net.minecraft.client.gui.GuiButton;

/**
 * Screens whose buttons only make sense with painted context (a toggle next
 * to a painted feature name, a glyph-only icon) give the agent's
 * {@code screen_tree} a self-describing label instead.
 */
public interface AgentLabeled {

    /** Label for {@code button}, or null for its {@code displayString}. */
    String agentLabel(GuiButton button);
}
