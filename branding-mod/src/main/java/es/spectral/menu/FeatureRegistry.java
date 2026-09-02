package es.spectral.menu;

import java.util.List;

/**
 * In-game feature registry matching Contract A.
 * Managed features map to third-party QoL jars (require restart).
 * Owned features apply live in-game.
 */
public final class FeatureRegistry {

    public record Feature(
            String id,
            String name,
            String description,
            String kind,
            boolean defaultEnabled,
            String defaultKeybind
    ) {
        public boolean isManaged() {
            return "managed".equalsIgnoreCase(kind);
        }

        public boolean isOwned() {
            return "owned".equalsIgnoreCase(kind);
        }
    }

    public static final List<Feature> ALL = List.of(
            new Feature("fullbright", "Fullbright", "Aumenta el brillo máximo del juego", "managed", true, null),
            new Feature("nofog", "No Fog", "Desactiva la niebla de distancia", "managed", true, null),
            new Feature("zoom", "Zoom", "Mantén Z para acercar la vista con suavizado (integrado)", "owned", true, "key.keyboard.z"),
            new Feature("macros", "Macros", "Atajos para chat y comandos", "owned", true, null)
    );

    private FeatureRegistry() {}

    public static Feature findById(String id) {
        if (id == null) return null;
        for (Feature f : ALL) {
            if (f.id().equalsIgnoreCase(id)) return f;
        }
        return null;
    }
}
