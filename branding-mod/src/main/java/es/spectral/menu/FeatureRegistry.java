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
            new Feature("fullbright", "Fullbright", "Brillo máximo en vivo sin guardar en disco (integrado)", "owned", true, null),
            new Feature("nofog", "No Fog", "Quita la niebla de distancia en vivo (integrado)", "owned", true, null),
            new Feature("zoom", "Zoom", "Mantén Z para acercar la vista con suavizado (integrado)", "owned", true, "key.keyboard.z"),
            new Feature("macros", "Macros", "Atajos para chat y comandos", "owned", true, null),
            new Feature("potionstatus", "Potion Status", "Efectos de poción activos con duración en pantalla (integrado)", "owned", false, null),
            new Feature("coords", "Coords", "Coordenadas XYZ y dirección en pantalla (integrado)", "owned", false, null),
            new Feature("healthstatus", "Salud", "Salud numérica y absorción en pantalla (integrado)", "owned", false, null),
            new Feature("armorstatus", "Armadura", "Durabilidad de cada pieza de armadura en pantalla (integrado)", "owned", false, null),
            new Feature("fpsping", "FPS / Ping", "Fotogramas por segundo y latencia en pantalla (integrado)", "owned", false, null),
            new Feature("lowfire", "Low Fire", "Baja el fuego al arder para ver mejor (integrado)", "owned", false, null),
            new Feature("clearwater", "Clear Water", "Quita el agua turbia y aleja la niebla submarina (integrado)", "owned", false, null),
            new Feature("chatheads", "Chat Heads", "Cabezas de jugador junto a los mensajes del chat (integrado)", "owned", false, null),
            new Feature("skin3d", "Capas 3D", "Convierte la segunda capa de la skin en vóxeles 3D (integrado)", "owned", false, null)
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
