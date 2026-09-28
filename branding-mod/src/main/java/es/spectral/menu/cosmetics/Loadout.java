package es.spectral.menu.cosmetics;

/**
 * Equipped cosmetics per slot. {@code null} means the slot is empty.
 * Slots follow {@code src/engine/cosmetics-catalog.json}: espalda, cuello,
 * cabeza, brazos, elitros.
 */
public record Loadout(String espalda, String cuello, String cabeza, String brazos, String elitros) {

    /** All-empty loadout; callers may render nothing. */
    public static final Loadout EMPTY = new Loadout(null, null, null, null, null);

    /** Item id equipped in {@code slot}, or null. Unknown slots return null. */
    public String forSlot(String slot) {
        if (slot == null) return null;
        return switch (slot) {
            case "espalda" -> espalda;
            case "cuello" -> cuello;
            case "cabeza" -> cabeza;
            case "brazos" -> brazos;
            case "elitros" -> elitros;
            default -> null;
        };
    }

    /** True when every slot is empty. */
    public boolean isEmpty() {
        return espalda == null && cuello == null && cabeza == null && brazos == null && elitros == null;
    }
}
