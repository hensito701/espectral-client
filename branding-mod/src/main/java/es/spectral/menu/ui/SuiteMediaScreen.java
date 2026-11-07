package es.spectral.menu.ui;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.components.AbstractSliderButton;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.StringWidget;
import net.minecraft.client.gui.components.Tooltip;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;

import es.spectral.menu.Compat;
import es.spectral.menu.media.MediaPaths;
import es.spectral.menu.media.MediaPlayer;
import es.spectral.menu.media.MediaToast;
import es.spectral.menu.media.MediaTrack;

/**
 * Media Player configuration: library, playlists and transport, opened from
 * the gear button on the Suite's Media Player row.
 *
 * <p>Two tabs share one screen so the 240&nbsp;px logical height stays
 * readable. Mode switches go through {@code rebuildWidgets()}, which clears the
 * widget list, so neither a tab change nor a resize can stack widgets; drafts of
 * the playlist name live in fields and survive a resize.
 *
 * <p>No seek bar: JavaMP3 has no cheap seek (its own {@code skip} decodes every
 * skipped frame), so the screen omits seeking rather than faking it — as the
 * spec allows. Elapsed/total are therefore a plain readout.
 */
public class SuiteMediaScreen extends Screen implements SuiteSheet {

    private enum Tab { LIBRARY, PLAYLISTS }

    private static final int ROW_H = 18;
    private static final int BUTTON_H = 18;
    private static final int FIELD_H = 18;
    private static final int HEADER_H = SuiteTheme.HEADER_H;

    private final Screen parent;
    private Tab tab = Tab.LIBRARY;
    private int page;
    private int pageSize = 4;
    private int lastPages = 1;

    private final List<Button> rowButtons = new ArrayList<>();
    private final List<MediaTrack> rowTracks = new ArrayList<>();
    private final List<Button> playlistButtons = new ArrayList<>();
    private StringWidget emptyLabel;
    private StringWidget headerLabel;
    private StringWidget nowPlayingLabel;
    private StringWidget countLabel;
    private Button pagerPrev;
    private Button pagerNext;
    private StringWidget pagerLabel;

    /** Selected playlist for the playlist tab (id), {@code null} = list view. */
    private String selectedPlaylist;
    /**
     * Track being added to a playlist. Non-null = "pick a playlist" mode: the
     * Playlists tab shows its list and a click on a row adds this track, then
     * returns to the Library. Back/cancel clears it.
     */
    private String pendingAddTrack;
    private String draftName;
    private EditBox nameBox;
    private boolean refreshQueued;

    public SuiteMediaScreen(Screen parent) {
        super(Component.translatable("espectral.media.title"));
        this.parent = parent;
    }

    @Override
    protected void init() {
        super.init();
        MediaPlayer player = MediaPlayer.get();
        player.reloadSettings();
        this.clearWidgets();
        this.rowButtons.clear();
        this.rowTracks.clear();
        this.playlistButtons.clear();

        int innerX = SuiteTheme.panelX(this.width) + SuiteTheme.MARGIN;
        int innerW = SuiteTheme.panelWidth(this.width) - 2 * SuiteTheme.MARGIN;
        int y = SuiteTheme.PANEL_TOP + SuiteTheme.PAD;

        this.headerLabel = new StringWidget(innerX, y, innerW, HEADER_H, Component.empty(), this.font);
        this.addRenderableWidget(this.headerLabel);
        y += HEADER_H + 2;

        // Tab chips.
        int halfW = (innerW - SuiteTheme.GAP) / 2;
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.tab.library"),
                b -> {
                    this.tab = Tab.LIBRARY;
                    this.page = 0;
                    this.rebuildWidgets();
                }).bounds(innerX, y, halfW, SuiteTheme.CAT_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.tab.playlists"),
                b -> {
                    this.tab = Tab.PLAYLISTS;
                    this.selectedPlaylist = null;
                    this.page = 0;
                    this.rebuildWidgets();
                }).bounds(innerX + halfW + SuiteTheme.GAP, y, halfW, SuiteTheme.CAT_H).build());
        y += SuiteTheme.CAT_H + 2;

        int footerY = SuiteTheme.footerY(this.height);
        int rowsBottom = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H - 2;

        // Now-playing line + settings row live above the list on both tabs.
        this.nowPlayingLabel = new StringWidget(innerX, y, innerW, 12, Component.empty(), this.font);
        this.addRenderableWidget(this.nowPlayingLabel);
        y += 12;

        int ctlW = (innerW - 3 * SuiteTheme.GAP) / 4;
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.prev"),
                b -> MediaPlayer.get().previous()).bounds(innerX, y, ctlW, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.play"),
                b -> MediaPlayer.get().togglePlayPause())
                .bounds(innerX + ctlW + SuiteTheme.GAP, y, ctlW, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.next"),
                b -> MediaPlayer.get().next())
                .bounds(innerX + 2 * (ctlW + SuiteTheme.GAP), y, ctlW, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.literal(repeatLabel()),
                b -> {
                    MediaPlayer.get().cycleRepeat();
                    b.setMessage(Component.literal(repeatLabel()));
                }).bounds(innerX + 3 * (ctlW + SuiteTheme.GAP), y, ctlW, BUTTON_H).build());
        y += BUTTON_H + 2;

        this.addRenderableWidget(new VolumeSlider(innerX, y, innerW, BUTTON_H,
                MediaPlayer.get().volume()));
        y += BUTTON_H + 2;

        this.addRenderableWidget(Button.builder(
                Component.translatable(MediaPlayer.get().shuffle()
                        ? "espectral.media.shuffle.on" : "espectral.media.shuffle"),
                b -> {
                    MediaPlayer.get().toggleShuffle();
                    b.setMessage(Component.translatable(MediaPlayer.get().shuffle()
                            ? "espectral.media.shuffle.on" : "espectral.media.shuffle"));
                }).bounds(innerX, y, (innerW - 2 * SuiteTheme.GAP) / 3, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(
                Component.translatable(MediaPlayer.get().duckMusic()
                        ? "espectral.media.duck.on" : "espectral.media.duck.off"),
                b -> {
                    MediaPlayer.get().setDuckMusic(!MediaPlayer.get().duckMusic());
                    b.setMessage(Component.translatable(MediaPlayer.get().duckMusic()
                            ? "espectral.media.duck.on" : "espectral.media.duck.off"));
                }).bounds(innerX + (innerW - 2 * SuiteTheme.GAP) / 3 + SuiteTheme.GAP, y,
                        (innerW - 2 * SuiteTheme.GAP) / 3, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(
                Component.translatable("espectral.media.refresh"), b -> {
                    this.refreshQueued = true;
                    MediaPlayer.get().refreshLibrary(() -> this.refreshQueued = false);
                }).bounds(innerX + 2 * ((innerW - 2 * SuiteTheme.GAP) / 3 + SuiteTheme.GAP), y,
                        (innerW - 2 * SuiteTheme.GAP) / 3, BUTTON_H).build());
        y += BUTTON_H + 2;

        int rowsTop = y;
        if (this.tab == Tab.LIBRARY) {
            initLibrary(innerX, innerW, rowsTop, rowsBottom, footerY);
        } else {
            initPlaylists(innerX, innerW, rowsTop, rowsBottom, footerY);
        }
        this.refresh();
        if (!MediaPlayer.get().library().isScanning()
                && MediaPlayer.get().library().tracks().isEmpty()) {
            MediaPlayer.get().refreshLibrary(() -> this.refreshQueued = false);
        }
    }

    private void initLibrary(int innerX, int innerW, int rowsTop, int rowsBottom, int footerY) {
        this.pageSize = Math.max(1, (rowsBottom - rowsTop) / ROW_H);
        for (int i = 0; i < this.pageSize; i++) {
            final int slot = i;
            Button row = Button.builder(Component.empty(), b -> {
                if (slot < this.rowTracks.size()) {
                    MediaPlayer.get().playQueue("", allIds());
                    MediaPlayer.get().playTrack(this.rowTracks.get(slot));
                }
            }).bounds(innerX, rowsTop + i * ROW_H, innerW - ROW_H, ROW_H - 2).build();
            this.rowButtons.add(row);
            this.addRenderableWidget(row);
            // "+": pick a playlist for this track (no separate playlist editor
            // entry point per track, so this is how playlists get filled).
            this.addRenderableWidget(Button.builder(Component.literal("+"), b -> {
                if (slot < this.rowTracks.size()) {
                    this.pendingAddTrack = this.rowTracks.get(slot).id();
                    this.tab = Tab.PLAYLISTS;
                    this.selectedPlaylist = null;
                    this.page = 0;
                    this.rebuildWidgets();
                }
            }).bounds(innerX + innerW - ROW_H + 2, rowsTop + i * ROW_H, ROW_H - 2, ROW_H - 2)
                    .tooltip(Tooltip.create(Component.translatable("espectral.media.addto"))).build());
        }
        this.emptyLabel = new StringWidget(innerX, rowsTop + 2, innerW, 12,
                Component.empty(), this.font);
        this.addRenderableWidget(this.emptyLabel);
        initPagerAndFooter(innerX, innerW, footerY, true);
    }

    private void initPlaylists(int innerX, int innerW, int rowsTop, int rowsBottom, int footerY) {
        this.pageSize = Math.max(1, (rowsBottom - rowsTop) / ROW_H);
        MediaPlayer.Playlist playlist = MediaPlayer.get().playlistById(this.selectedPlaylist);
        if (playlist == null) {
            // Playlist list: one row per playlist.
            for (int i = 0; i < this.pageSize; i++) {
                final int slot = i;
                Button row = Button.builder(Component.empty(), b -> {
                    List<MediaPlayer.Playlist> all = MediaPlayer.get().playlists();
                    int index = this.page * this.pageSize + slot;
                    if (index >= all.size()) {
                        return;
                    }
                    MediaPlayer.Playlist target = all.get(index);
                    if (this.pendingAddTrack != null) {
                        // Pick mode: add and go back to the library.
                        MediaPlayer.get().addToPlaylist(target.id(), this.pendingAddTrack);
                        this.pendingAddTrack = null;
                        this.tab = Tab.LIBRARY;
                        this.page = 0;
                        this.rebuildWidgets();
                        return;
                    }
                    this.selectedPlaylist = target.id();
                    this.draftName = target.name();
                    this.page = 0;
                    this.rebuildWidgets();
                }).bounds(innerX, rowsTop + i * ROW_H, innerW, ROW_H - 2).build();
                this.playlistButtons.add(row);
                this.addRenderableWidget(row);
            }
            this.countLabel = new StringWidget(innerX, rowsTop + 2, innerW, 12,
                    Component.empty(), this.font);
            this.addRenderableWidget(this.countLabel);
            initPagerAndFooter(innerX, innerW, footerY, false);
            return;
        }

        // Playlist detail: tracks with up/down/remove controls.
        this.nameBox = new EditBox(this.font, innerX, rowsTop, innerW - 60, FIELD_H,
                Component.translatable("espectral.media.rename"));
        this.nameBox.setValue(this.draftName == null ? playlist.name() : this.draftName);
        this.nameBox.setMaxLength(40);
        this.nameBox.setResponder(value -> this.draftName = value);
        this.addRenderableWidget(this.nameBox);
        this.addRenderableWidget(Button.builder(Component.literal("\u2713"), b -> {
            MediaPlayer.get().renamePlaylist(playlist.id(), this.draftName);
            this.rebuildWidgets();
        }).bounds(innerX + innerW - 56, rowsTop, 56, FIELD_H).build());
        int tracksTop = rowsTop + FIELD_H + 2;

        int detailRows = Math.max(1, (rowsBottom - tracksTop) / ROW_H);
        for (int i = 0; i < detailRows; i++) {
            final int slot = i;
            Button row = Button.builder(Component.empty(), b -> {
                MediaPlayer.get().playQueue(playlist.id(), playlist.tracks());
            }).bounds(innerX, tracksTop + i * ROW_H, innerW - 56, ROW_H - 2).build();
            this.playlistButtons.add(row);
            this.addRenderableWidget(row);
            final int index = this.page * detailRows + slot;
            int bx = innerX + innerW - 54;
            this.addRenderableWidget(Button.builder(Component.literal("\u25B2"),
                    b -> MediaPlayer.get().moveInPlaylist(playlist.id(), index, -1))
                    .bounds(bx, tracksTop + i * ROW_H, 18, ROW_H - 2).build());
            this.addRenderableWidget(Button.builder(Component.literal("\u25BC"),
                    b -> MediaPlayer.get().moveInPlaylist(playlist.id(), index, 1))
                    .bounds(bx + 18, tracksTop + i * ROW_H, 18, ROW_H - 2).build());
            this.addRenderableWidget(Button.builder(Component.literal("\u2715"),
                    b -> {
                        if (index < playlist.tracks().size()) {
                            MediaPlayer.get().removeFromPlaylist(playlist.id(),
                                    playlist.tracks().get(index));
                        }
                        this.rebuildWidgets();
                    }).bounds(bx + 36, tracksTop + i * ROW_H, 18, ROW_H - 2).build());
        }
        this.emptyLabel = new StringWidget(innerX, tracksTop + 2, innerW, 12,
                Component.translatable("espectral.media.playlist.empty"), this.font);
        this.addRenderableWidget(this.emptyLabel);
        this.countLabel = null;
        this.pagerPrev = null;
        this.pagerNext = null;
        this.pagerLabel = null;

        int bw = (innerW - 2 * SuiteTheme.GAP) / 3;
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.play"),
                b -> MediaPlayer.get().playQueue(playlist.id(), playlist.tracks()))
                .bounds(innerX, footerY, bw, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.suite.back"),
                b -> {
                    this.selectedPlaylist = null;
                    this.rebuildWidgets();
                }).bounds(innerX + bw + SuiteTheme.GAP, footerY, bw, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.delete"),
                b -> {
                    MediaPlayer.get().deletePlaylist(playlist.id());
                    this.selectedPlaylist = null;
                    this.rebuildWidgets();
                }).bounds(innerX + 2 * (bw + SuiteTheme.GAP), footerY, bw, BUTTON_H)
                .tooltip(Tooltip.create(Component.translatable("espectral.media.delete.message",
                        playlist.name()))).build());
    }

    /** Leaves pick mode (used by Back and by opening the detail view). */
    private void cancelPick() {
        if (this.pendingAddTrack == null) {
            return;
        }
        this.pendingAddTrack = null;
        this.tab = Tab.LIBRARY;
        this.page = 0;
        this.rebuildWidgets();
    }

    private void initPagerAndFooter(int innerX, int innerW, int footerY, boolean library) {
        int pagerY = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H;
        int pagerW = 40;
        this.pagerPrev = Button.builder(Component.translatable("espectral.suite.button.prev"), b -> {
            this.page = Math.max(0, this.page - 1);
            this.refresh();
        }).bounds(innerX, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerPrev);
        this.pagerNext = Button.builder(Component.translatable("espectral.suite.button.next"), b -> {
            this.page = Math.min(this.lastPages - 1, this.page + 1);
            this.refresh();
        }).bounds(innerX + innerW - pagerW, pagerY, pagerW, SuiteTheme.PAGER_H).build();
        this.addRenderableWidget(this.pagerNext);
        this.pagerLabel = new StringWidget(innerX + pagerW + SuiteTheme.GAP, pagerY,
                innerW - 2 * (pagerW + SuiteTheme.GAP), SuiteTheme.PAGER_H, Component.empty(), this.font);
        this.addRenderableWidget(this.pagerLabel);

        int bw = (innerW - 2 * SuiteTheme.GAP) / 3;
        this.addRenderableWidget(Button.builder(
                Component.translatable(library ? "espectral.media.openfolder" : "espectral.media.new"),
                b -> {
                    if (library) {
                        MediaPaths.openFolder();
                    } else {
                        MediaPlayer.Playlist created =
                                MediaPlayer.get().createPlaylist(
                                        Component.translatable("espectral.media.newlist",
                                                MediaPlayer.get().playlists().size() + 1).getString());
                        if (created != null) {
                            this.selectedPlaylist = created.id();
                            this.draftName = created.name();
                        }
                        this.rebuildWidgets();
                    }
                }).bounds(innerX, footerY, bw, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.suite.back"),
                b -> {
                    if (this.pendingAddTrack != null) {
                        this.cancelPick();
                    } else {
                        this.onClose();
                    }
                })
                .bounds(innerX + bw + SuiteTheme.GAP, footerY, bw, BUTTON_H).build());
        this.addRenderableWidget(Button.builder(Component.translatable("espectral.media.credits"),
                b -> { }).bounds(innerX + 2 * (bw + SuiteTheme.GAP), footerY, bw, BUTTON_H).build());
    }

    private List<String> allIds() {
        List<String> ids = new ArrayList<>();
        for (MediaTrack track : MediaPlayer.get().library().tracks()) {
            ids.add(track.id());
        }
        return ids;
    }

    private static String repeatLabel() {
        return switch (MediaPlayer.get().repeat()) {
            case MediaPlayer.REPEAT_ALL -> Component.translatable("espectral.media.repeat.all").getString();
            case MediaPlayer.REPEAT_ONE -> Component.translatable("espectral.media.repeat.one").getString();
            default -> Component.translatable("espectral.media.repeat.off").getString();
        };
    }

    @Override
    public void tick() {
        super.tick();
        if (this.refreshQueued) {
            this.refresh();
        }
        if (this.nowPlayingLabel != null) {
            this.nowPlayingLabel.setMessage(nowPlayingLine());
        }
    }

    private MutableComponent nowPlayingLine() {
        MediaPlayer player = MediaPlayer.get();
        MediaTrack track = player.current();
        if (track == null) {
            return Component.translatable("espectral.media.nowplaying.none");
        }
        String clock = MediaToast.clock(player.elapsedMs()) + " / "
                + (track.durationMs() > 0 ? MediaToast.clock(track.durationMs()) : "--:--");
        return Component.translatable("espectral.media.nowplaying")
                .append(Component.literal(": " + track.displayName() + "  (" + clock + ")"));
    }

    /** Repaints messages in place; adds and removes nothing. */
    private void refresh() {
        MediaPlayer player = MediaPlayer.get();
        if (this.headerLabel != null) {
            this.headerLabel.setMessage(this.pendingAddTrack == null
                    ? Component.translatable("espectral.media.title")
                    : Component.translatable("espectral.media.addto"));
        }
        if (this.tab == Tab.LIBRARY) {
            List<MediaTrack> tracks = player.library().tracks();
            this.lastPages = Math.max(1, (tracks.size() + this.pageSize - 1) / this.pageSize);
            this.page = Math.min(Math.max(0, this.page), this.lastPages - 1);
            this.rowTracks.clear();
            for (int i = this.page * this.pageSize;
                    i < tracks.size() && this.rowTracks.size() < this.pageSize; i++) {
                this.rowTracks.add(tracks.get(i));
            }
            for (int i = 0; i < this.rowButtons.size(); i++) {
                Button button = this.rowButtons.get(i);
                if (i < this.rowTracks.size()) {
                    MediaTrack track = this.rowTracks.get(i);
                    button.setMessage(Component.literal(track.displayName()));
                    button.setTooltip(Tooltip.create(Component.literal(track.id())));
                    button.visible = true;
                } else {
                    button.visible = false;
                }
            }
            this.emptyLabel.visible = tracks.isEmpty();
            this.emptyLabel.setMessage(Component.translatable(player.library().isScanning()
                    ? "espectral.media.scanning" : "espectral.media.empty"));
        } else {
            MediaPlayer.Playlist playlist = player.playlistById(this.selectedPlaylist);
            List<MediaPlayer.Playlist> all = player.playlists();
            if (playlist == null) {
                this.lastPages = Math.max(1, (all.size() + this.pageSize - 1) / this.pageSize);
                this.page = Math.min(Math.max(0, this.page), this.lastPages - 1);
                for (int i = 0; i < this.playlistButtons.size(); i++) {
                    Button button = this.playlistButtons.get(i);
                    int index = this.page * this.pageSize + i;
                    if (index < all.size()) {
                        MediaPlayer.Playlist entry = all.get(index);
                        button.setMessage(Component.translatable("espectral.media.playlist.tracks",
                                entry.name(), entry.tracks().size()));
                        button.visible = true;
                    } else {
                        button.visible = false;
                    }
                }
                if (this.countLabel != null) {
                    this.countLabel.visible = all.isEmpty();
                    this.countLabel.setMessage(Component.translatable("espectral.media.playlist.empty"));
                }
                if (this.emptyLabel != null) {
                    this.emptyLabel.visible = false;
                }
            } else {
                List<String> ids = playlist.tracks();
                int rows = this.playlistButtons.size();
                for (int i = 0; i < rows; i++) {
                    Button button = this.playlistButtons.get(i);
                    int index = this.page * rows + i;
                    if (index < ids.size()) {
                        MediaTrack track = player.library().byId(ids.get(index));
                        button.setMessage(Component.literal(track != null
                                ? track.displayName() : ids.get(index) + " \u26A0"));
                        button.visible = true;
                    } else {
                        button.visible = false;
                    }
                }
                if (this.emptyLabel != null) {
                    this.emptyLabel.visible = ids.isEmpty();
                }
            }
        }
        boolean paged = this.lastPages > 1 && this.pagerPrev != null;
        if (this.pagerPrev != null) {
            this.pagerPrev.visible = paged;
            this.pagerNext.visible = paged;
            this.pagerLabel.visible = paged;
            if (paged) {
                this.pagerLabel.setMessage(Component.translatable("espectral.suite.page",
                        this.page + 1, this.lastPages));
            }
        }
    }

    @Override
    public void paintSheet(Object gfx, int width, int height, Font font) {
        SuitePainter.paintSheet(gfx, width, height);
        int innerX = SuiteTheme.panelX(width) + SuiteTheme.MARGIN;
        int innerW = SuiteTheme.panelWidth(width) - 2 * SuiteTheme.MARGIN;
        // Folder hint sits just above the pager: the list has the middle band.
        int y = SuiteTheme.footerY(height) - SuiteTheme.GAP - SuiteTheme.PAGER_H - 11;
        Compat.uiText(gfx, font, Component.literal(MediaPaths.musicDir().toString()),
                innerX, y, SuiteTheme.TEXT_DIM, false);
    }

    @Override
    public void onClose() {
        Minecraft minecraft = Minecraft.getInstance();
        if (this.parent != null && minecraft != null) {
            Compat.open(minecraft, this.parent);
            return;
        }
        super.onClose();
    }

    /** Volume slider bound to the media volume (independent of the game's). */
    private static final class VolumeSlider extends AbstractSliderButton {

        VolumeSlider(int x, int y, int width, int height, double initial) {
            super(x, y, width, height, Component.empty(), initial);
            updateMessage();
        }

        @Override
        protected void updateMessage() {
            setMessage(Component.translatable("espectral.media.volume")
                    .append(Component.literal(": " + Math.round(this.value * 100.0) + "%")));
        }

        @Override
        protected void applyValue() {
            MediaPlayer.get().setVolume((float) this.value);
        }
    }
}
