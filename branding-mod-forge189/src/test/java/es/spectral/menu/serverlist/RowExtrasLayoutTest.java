package es.spectral.menu.serverlist;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

/**
 * Row extras geometry and ping colours: the two pieces of this feature that are
 * plain arithmetic, so they are checked without a running client.
 */
public class RowExtrasLayoutTest {

    @Test
    public void resetForgetsEverySlot() {
        RowExtrasLayout layout = new RowExtrasLayout();
        layout.place(RowExtrasLayout.SLOT_PING, 100, 4, 30, RowExtras.SLOT_H);
        layout.place(RowExtrasLayout.SLOT_FLAG, 70, 4, RowExtras.FLAG_W, RowExtras.FLAG_H);
        assertTrue(layout.isPlaced(RowExtrasLayout.SLOT_PING));
        assertTrue(layout.isPlaced(RowExtrasLayout.SLOT_FLAG));

        layout.reset();
        assertFalse(layout.isPlaced(RowExtrasLayout.SLOT_PING));
        assertFalse(layout.isPlaced(RowExtrasLayout.SLOT_FLAG));
        assertEquals(-1, layout.hit(100, 4));
    }

    @Test
    public void hitUsesHalfOpenRectangles() {
        RowExtrasLayout layout = new RowExtrasLayout();
        layout.place(RowExtrasLayout.SLOT_FLAG, 70, 4, 16, 11);

        assertEquals(RowExtrasLayout.SLOT_FLAG, layout.hit(70, 4));
        assertEquals(RowExtrasLayout.SLOT_FLAG, layout.hit(85, 14));
        assertEquals(-1, layout.hit(86, 4));
        assertEquals(-1, layout.hit(70, 15));
        assertEquals(-1, layout.hit(69, 10));
    }

    @Test
    public void zeroSizedPlacementsDoNotRegister() {
        RowExtrasLayout layout = new RowExtrasLayout();
        layout.place(RowExtrasLayout.SLOT_EXTRA, 10, 10, 0, 11);
        assertFalse(layout.isPlaced(RowExtrasLayout.SLOT_EXTRA));
        assertEquals(-1, layout.hit(10, 10));
    }

    @Test
    public void pingColourBands() {
        assertEquals(RowExtras.COLOR_GOOD, RowExtras.pingColor(0));
        assertEquals(RowExtras.COLOR_GOOD, RowExtras.pingColor(79));
        assertEquals(RowExtras.COLOR_WARN, RowExtras.pingColor(80));
        assertEquals(RowExtras.COLOR_WARN, RowExtras.pingColor(149));
        assertEquals(RowExtras.COLOR_BAD, RowExtras.pingColor(150));
        assertEquals(RowExtras.COLOR_BAD, RowExtras.pingColor(9999));
        assertEquals(RowExtras.COLOR_BAD, RowExtras.pingColor(-1));
    }
}
