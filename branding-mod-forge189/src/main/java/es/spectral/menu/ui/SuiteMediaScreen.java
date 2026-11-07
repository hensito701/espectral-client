package es.spectral.menu.ui;

import java.util.ArrayList;
import java.util.List;

import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.gui.GuiTextField;

import org.lwjgl.input.Keyboard;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Lang;
import es.spectral.menu.media.MediaPaths;
import es.spectral.menu.media.MediaPlayer;
import es.spectral.menu.media.MediaPlayer.Playlist;
import es.spectral.menu.media.MediaToast;
import es.spectral.menu.media.MediaTrack;

/**
 * Media Player screen, opened from the gear of the Suite's MediaPlayer row.
 * Library of the music folder, playlists, and transport. Mirrors the Fabric
 * screen; the playback itself lives in {@link MediaPlayer}.
 */
public class SuiteMediaScreen extends GuiScreen {

    private enum Tab { LIBRARY, PLAYLISTS }

    private static final int FIELD_H = 18;
    private static final int TRANSPORT_H = 18;
    private static final int TABS_Y = 30;
    private static final int NOW_Y = 47;
    private static final int TRANSPORT_Y = 58;
    private static final int CONTROLS_Y = 80;
    private static final int LIST_TOP = 102;
    private static final int SIDE_W = 40;
    private static final int VOLUME_W = 20;
    private static final int PAGER_W = 40;
    private static final int RENAME_W = 56;
    private static final int DETAIL_BUTTON_W = 18;
    private static final int ROW_SLOTS = 100;
    private static final float VOLUME_STEP = 0.05F;

    private static final int TAB_LIBRARY = 1;
    private static final int TAB_PLAYLISTS = 2;
    private static final int PREV_TRACK = 3;
    private static final int PLAY = 4;
    private static final int NEXT_TRACK = 5;
    private static final int REPEAT = 6;
    private static final int SHUFFLE = 7;
    private static final int DUCK = 8;
    private static final int REFRESH = 9;
    private static final int PAGE_PREV = 10;
    private static final int PAGE_NEXT = 11;
    private static final int OPEN_FOLDER = 12;
    private static final int BACK = 13;
    private static final int VOLUME_DOWN = 14;
    private static final int VOLUME_UP = 15;
    private static final int RENAME = 16;
    private static final int DETAIL_PLAY = 17;
    private static final int DELETE = 18;
    private static final int NEW_PLAYLIST = 19;
    private static final int ID_ROW = 100;
    private static final int ID_UP = 200;
    private static final int ID_DOWN = 300;
    private static final int ID_REMOVE = 400;
    private static final int ID_ADD = 500;

    private final GuiScreen parent;
    private final MediaPlayer player = MediaPlayer.get();
    /** One id per visible row slot: track ids in the library and in a playlist, playlist ids in the list. */
    private final List<String> rowKeys = new ArrayList<String>();
    private Tab tab = Tab.LIBRARY;
    /** Playlist open in the detail view, or null for the list. */
    private String selectedPlaylist;
    /** Track waiting for a playlist to be picked (add-to mode), or null. */
    private String pendingAddTrack;
    private String draftName = "";
    private int page;
    private int pageSize = 1;
    private int lastPages = 1;
    private int innerX;
    private int innerW;
    private int pagerY;
    private int creditsY;
    private int volumeLeft;
    private int volumeRight;
    private boolean wasScanning;
    private GuiTextField nameBox;
    private SuiteButton playButton;
    private SuiteButton repeatButton;
    private SuiteButton shuffleButton;
    private SuiteButton duckButton;
    private SuiteButton refreshButton;

    public SuiteMediaScreen(GuiScreen parent) {
        this.parent = parent;
    }

    @Override
    public void initGui() {
        Keyboard.enableRepeatEvents(true);
        buttonList.clear();
        rowKeys.clear();
        nameBox = null;
        refreshButton = null;
        innerX = SuiteTheme.panelX(width) + SuiteTheme.MARGIN;
        innerW = SuiteTheme.panelWidth(width) - 2 * SuiteTheme.MARGIN;
        int footerY = SuiteTheme.footerY(height);
        pagerY = footerY - SuiteTheme.GAP - SuiteTheme.PAGER_H;
        creditsY = pagerY - 10;

        Playlist pl = selectedPlaylist == null ? null : player.playlistById(selectedPlaylist);
        if (pl == null) {
            selectedPlaylist = null;
        }
        initTransport();
        initTabs();
        if (tab == Tab.LIBRARY) {
            initLibrary(footerY);
        } else if (pl == null) {
            initPlaylists(footerY);
        } else {
            initDetail(pl, footerY);
        }
        syncLabels();
        wasScanning = player.library().isScanning();
    }

    private void initTransport() {
        int gap = SuiteTheme.GAP;
        buttonList.add(new SuiteButton(PREV_TRACK, innerX, TRANSPORT_Y, SIDE_W, TRANSPORT_H, "|<"));
        playButton = new SuiteButton(PLAY, innerX + SIDE_W + gap, TRANSPORT_Y, innerW - 2 * (SIDE_W + gap), TRANSPORT_H, "");
        buttonList.add(playButton);
        buttonList.add(new SuiteButton(NEXT_TRACK, innerX + innerW - SIDE_W, TRANSPORT_Y, SIDE_W, TRANSPORT_H, ">|"));

        int ctl = Math.max(36, (innerW - 150) / 3);
        int step = ctl + gap;
        shuffleButton = new SuiteButton(SHUFFLE, innerX, CONTROLS_Y, ctl, TRANSPORT_H, "");
        repeatButton = new SuiteButton(REPEAT, innerX + step, CONTROLS_Y, ctl, TRANSPORT_H, "");
        repeatButton.tooltip = Lang.tr("espectral.media.repeat");
        duckButton = new SuiteButton(DUCK, innerX + 2 * step, CONTROLS_Y, ctl, TRANSPORT_H, "");
        duckButton.tooltip = Lang.tr("espectral.media.duck");
        buttonList.add(shuffleButton);
        buttonList.add(repeatButton);
        buttonList.add(duckButton);

        int volDownX = innerX + 3 * step;
        SuiteButton down = new SuiteButton(VOLUME_DOWN, volDownX, CONTROLS_Y, VOLUME_W, TRANSPORT_H, "-");
        down.tooltip = Lang.tr("espectral.media.volume");
        SuiteButton up = new SuiteButton(VOLUME_UP, innerX + innerW - VOLUME_W, CONTROLS_Y, VOLUME_W, TRANSPORT_H, "+");
        up.tooltip = Lang.tr("espectral.media.volume");
        buttonList.add(down);
        buttonList.add(up);
        volumeLeft = volDownX + VOLUME_W + gap;
        volumeRight = innerX + innerW - VOLUME_W - gap;
    }

    private void initTabs() {
        int half = (innerW - SuiteTheme.GAP) / 2;
        SuiteButton library = new SuiteButton(TAB_LIBRARY, innerX, TABS_Y, half, SuiteTheme.CAT_H, Lang.tr("espectral.media.tab.library"));
        library.selected = tab == Tab.LIBRARY;
        SuiteButton playlists = new SuiteButton(TAB_PLAYLISTS, innerX + half + SuiteTheme.GAP, TABS_Y,
                innerW - half - SuiteTheme.GAP, SuiteTheme.CAT_H, Lang.tr("espectral.media.tab.playlists"));
        playlists.selected = tab == Tab.PLAYLISTS;
        buttonList.add(library);
        buttonList.add(playlists);
    }

    private void initLibrary(int footerY) {
        List<MediaTrack> all = player.library().tracks();
        paginate(all.size(), LIST_TOP);
        int start = page * pageSize;
        int end = Math.min(all.size(), start + pageSize);
        int rowW = innerW - SuiteTheme.ROW_H;
        int addX = innerX + innerW - (SuiteTheme.ROW_H - 2);
        for (int i = start; i < end; i++) {
            int slot = i - start;
            MediaTrack track = all.get(i);
            int y = LIST_TOP + slot * SuiteTheme.ROW_H;
            rowKeys.add(track.id());
            SuiteButton row = new SuiteButton(ID_ROW + slot, innerX, y, rowW, SuiteTheme.ROW_H - 2,
                    fit(track.displayName(), rowW - 8));
            row.tooltip = track.id();
            buttonList.add(row);
            SuiteButton add = new SuiteButton(ID_ADD + slot, addX, y, SuiteTheme.ROW_H - 2, SuiteTheme.ROW_H - 2, "+");
            add.tooltip = Lang.tr("espectral.media.addto");
            buttonList.add(add);
        }
        initPager();
        SuiteButton folder = addFooter(0, 3, footerY, OPEN_FOLDER, Lang.tr("espectral.media.openfolder"));
        folder.tooltip = Lang.tr("espectral.media.folder.hint") + "  " + MediaPaths.musicDir().toAbsolutePath();
        refreshButton = addFooter(1, 3, footerY, REFRESH, Lang.tr("espectral.media.refresh"));
        addFooter(2, 3, footerY, BACK, Lang.tr("espectral.suite.back"));
    }

    private void initPlaylists(int footerY) {
        List<Playlist> all = player.playlists();
        paginate(all.size(), LIST_TOP);
        int start = page * pageSize;
        int end = Math.min(all.size(), start + pageSize);
        for (int i = start; i < end; i++) {
            int slot = i - start;
            Playlist pl = all.get(i);
            rowKeys.add(pl.id());
            String label = pl.name() + " - " + Lang.tr("espectral.media.playlist.tracks", pl.tracks().size());
            buttonList.add(new SuiteButton(ID_ROW + slot, innerX, LIST_TOP + slot * SuiteTheme.ROW_H, innerW,
                    SuiteTheme.ROW_H - 2, fit(label, innerW - 8)));
        }
        initPager();
        addFooter(0, 2, footerY, NEW_PLAYLIST, Lang.tr("espectral.media.new"));
        addFooter(1, 2, footerY, BACK, Lang.tr("espectral.suite.back"));
    }

    private void initDetail(Playlist pl, int footerY) {
        int boxW = innerW - RENAME_W - 4;
        nameBox = new GuiTextField(0, fontRendererObj, innerX + 4, LIST_TOP + 5, boxW - 8, 10);
        nameBox.setEnableBackgroundDrawing(false);
        nameBox.setMaxStringLength(ClientConfig.MAX_PLAYLIST_NAME);
        nameBox.setText(draftName);
        nameBox.setFocused(true);
        SuiteButton rename = new SuiteButton(RENAME, innerX + innerW - RENAME_W, LIST_TOP, RENAME_W, FIELD_H, "OK");
        rename.tooltip = Lang.tr("espectral.media.rename");
        buttonList.add(rename);

        int rowsTop = LIST_TOP + SuiteTheme.ROW_H;
        List<String> ids = pl.tracks();
        paginate(ids.size(), rowsTop);
        int start = page * pageSize;
        int end = Math.min(ids.size(), start + pageSize);
        int rowW = innerW - RENAME_W;
        int bx = innerX + innerW - 3 * DETAIL_BUTTON_W;
        for (int i = start; i < end; i++) {
            int slot = i - start;
            String id = ids.get(i);
            int y = rowsTop + slot * SuiteTheme.ROW_H;
            MediaTrack track = player.library().byId(id);
            rowKeys.add(id);
            buttonList.add(new SuiteButton(ID_ROW + slot, innerX, y, rowW, SuiteTheme.ROW_H - 2,
                    fit(track == null ? id : track.displayName(), rowW - 8)));
            SuiteButton up = new SuiteButton(ID_UP + slot, bx, y, DETAIL_BUTTON_W, SuiteTheme.ROW_H - 2, "^");
            up.enabled = i > 0;
            up.tooltip = Lang.tr("espectral.media.up");
            SuiteButton down = new SuiteButton(ID_DOWN + slot, bx + DETAIL_BUTTON_W, y, DETAIL_BUTTON_W,
                    SuiteTheme.ROW_H - 2, "v");
            down.enabled = i < ids.size() - 1;
            down.tooltip = Lang.tr("espectral.media.down");
            SuiteButton remove = new SuiteButton(ID_REMOVE + slot, bx + 2 * DETAIL_BUTTON_W, y, DETAIL_BUTTON_W,
                    SuiteTheme.ROW_H - 2, "x");
            remove.tooltip = Lang.tr("espectral.media.remove");
            buttonList.add(up);
            buttonList.add(down);
            buttonList.add(remove);
        }
        initPager();
        addFooter(0, 3, footerY, DETAIL_PLAY, Lang.tr("espectral.media.play"));
        SuiteButton delete = addFooter(1, 3, footerY, DELETE, Lang.tr("espectral.media.delete"));
        delete.tooltip = Lang.tr("espectral.media.delete.message", pl.name());
        addFooter(2, 3, footerY, BACK, Lang.tr("espectral.suite.back"));
    }

    /** Sizes the page from the space between {@code rowsTop} and the credits line, then clamps {@link #page}. */
    private void paginate(int count, int rowsTop) {
        int rowsBottom = creditsY - SuiteTheme.GAP;
        pageSize = Math.max(1, Math.min(ROW_SLOTS, (rowsBottom - rowsTop) / SuiteTheme.ROW_H));
        lastPages = Math.max(1, (count + pageSize - 1) / pageSize);
        page = Math.max(0, Math.min(page, lastPages - 1));
    }

    private void initPager() {
        if (lastPages <= 1) {
            return;
        }
        SuiteButton prev = new SuiteButton(PAGE_PREV, innerX, pagerY, PAGER_W, SuiteTheme.PAGER_H,
                Lang.tr("espectral.suite.button.prev"));
        prev.enabled = page > 0;
        SuiteButton next = new SuiteButton(PAGE_NEXT, innerX + innerW - PAGER_W, pagerY, PAGER_W, SuiteTheme.PAGER_H,
                Lang.tr("espectral.suite.button.next"));
        next.enabled = page < lastPages - 1;
        buttonList.add(prev);
        buttonList.add(next);
    }

    private SuiteButton addFooter(int index, int count, int footerY, int id, String label) {
        int w = (innerW - (count - 1) * SuiteTheme.GAP) / count;
        SuiteButton button = new SuiteButton(id, innerX + index * (w + SuiteTheme.GAP), footerY, w,
                SuiteTheme.FOOTER_H, label);
        buttonList.add(button);
        return button;
    }

    /** Keeps the dynamic labels and enabled states in step with the player. */
    private void syncLabels() {
        boolean playing = player.isPlaying() && !player.isPaused();
        playButton.displayString = Lang.tr(playing ? "espectral.media.pause" : "espectral.media.play");
        String repeat = player.repeat();
        String repeatKey = MediaPlayer.REPEAT_ONE.equals(repeat) ? "espectral.media.repeat.one"
                : MediaPlayer.REPEAT_ALL.equals(repeat) ? "espectral.media.repeat.all" : "espectral.media.repeat.off";
        repeatButton.displayString = Lang.tr(repeatKey);
        shuffleButton.displayString = Lang.tr(player.shuffle() ? "espectral.media.shuffle.on" : "espectral.media.shuffle");
        duckButton.displayString = Lang.tr(player.duckMusic() ? "espectral.media.duck.on" : "espectral.media.duck.off");
        if (refreshButton != null) {
            refreshButton.enabled = !player.library().isScanning();
        }
    }

    @Override
    public void updateScreen() {
        if (nameBox != null) {
            nameBox.updateCursorCounter();
        }
        // The library scan publishes its list on another thread; rebuild when it starts or ends.
        if (wasScanning != player.library().isScanning()) {
            initGui();
        } else {
            syncLabels();
        }
    }

    @Override
    protected void actionPerformed(GuiButton button) {
        if (nameBox != null) {
            draftName = nameBox.getText();
        }
        if (dispatch(button.id)) {
            initGui();
        }
    }

    /** Handles a button press. Returns true when the screen must rebuild its buttons. */
    private boolean dispatch(int id) {
        if (inSlot(id, ID_ROW)) {
            return rowClicked(id - ID_ROW);
        }
        if (inSlot(id, ID_UP)) {
            return moveTrack(id - ID_UP, -1);
        }
        if (inSlot(id, ID_DOWN)) {
            return moveTrack(id - ID_DOWN, 1);
        }
        if (inSlot(id, ID_REMOVE)) {
            return removeTrack(id - ID_REMOVE);
        }
        if (inSlot(id, ID_ADD)) {
            return addRow(id - ID_ADD);
        }
        switch (id) {
            case TAB_LIBRARY:
                return selectTab(Tab.LIBRARY);
            case TAB_PLAYLISTS:
                return selectTab(Tab.PLAYLISTS);
            case PREV_TRACK:
                player.previous();
                return true;
            case PLAY:
                player.togglePlayPause();
                return true;
            case NEXT_TRACK:
                player.next();
                return true;
            case SHUFFLE:
                player.toggleShuffle();
                return true;
            case REPEAT:
                player.cycleRepeat();
                return true;
            case DUCK:
                player.setDuckMusic(!player.duckMusic());
                return true;
            case VOLUME_DOWN:
                nudgeVolume(-VOLUME_STEP);
                return true;
            case VOLUME_UP:
                nudgeVolume(VOLUME_STEP);
                return true;
            case REFRESH:
                player.refreshLibrary(() -> { });
                return true;
            case OPEN_FOLDER:
                MediaPaths.openFolder();
                return false;
            case NEW_PLAYLIST:
                createPlaylist();
                return true;
            case PAGE_PREV:
                page = Math.max(0, page - 1);
                return true;
            case PAGE_NEXT:
                page = page + 1;
                return true;
            case RENAME:
                renameSelected();
                return true;
            case DETAIL_PLAY:
                playSelected();
                return true;
            case DELETE:
                deleteSelected();
                return true;
            case BACK:
                return back();
            default:
                return false;
        }
    }

    private static boolean inSlot(int id, int base) {
        return id >= base && id < base + ROW_SLOTS;
    }

    private boolean rowClicked(int slot) {
        if (slot >= rowKeys.size()) {
            return false;
        }
        String key = rowKeys.get(slot);
        if (tab == Tab.LIBRARY) {
            MediaTrack track = player.library().byId(key);
            if (track == null) {
                return false;
            }
            List<String> ids = new ArrayList<String>();
            for (MediaTrack t : player.library().tracks()) {
                ids.add(t.id());
            }
            player.playQueue("", ids);
            player.playTrack(track);
            return true;
        }
        if (selectedPlaylist == null) {
            Playlist pl = player.playlistById(key);
            if (pl == null) {
                return false;
            }
            if (pendingAddTrack != null) {
                player.addToPlaylist(pl.id(), pendingAddTrack);
                pendingAddTrack = null;
                tab = Tab.LIBRARY;
                page = 0;
            } else {
                openPlaylist(pl);
            }
            return true;
        }
        Playlist pl = player.playlistById(selectedPlaylist);
        MediaTrack track = player.library().byId(key);
        if (pl == null || track == null) {
            return false;
        }
        player.playQueue(pl.id(), pl.tracks());
        player.playTrack(track);
        return true;
    }

    private boolean selectTab(Tab next) {
        tab = next;
        selectedPlaylist = null;
        if (next == Tab.LIBRARY) {
            pendingAddTrack = null;
        }
        page = 0;
        return true;
    }

    private boolean addRow(int slot) {
        if (slot >= rowKeys.size()) {
            return false;
        }
        pendingAddTrack = rowKeys.get(slot);
        tab = Tab.PLAYLISTS;
        selectedPlaylist = null;
        page = 0;
        return true;
    }

    private void openPlaylist(Playlist pl) {
        selectedPlaylist = pl.id();
        draftName = pl.name();
        page = 0;
    }

    private void createPlaylist() {
        Playlist created = player.createPlaylist(Lang.tr("espectral.media.newlist", player.playlists().size() + 1));
        if (created != null && pendingAddTrack == null) {
            openPlaylist(created);
        }
    }

    private boolean moveTrack(int slot, int delta) {
        if (selectedPlaylist == null || slot >= rowKeys.size()) {
            return false;
        }
        player.moveInPlaylist(selectedPlaylist, page * pageSize + slot, delta);
        return true;
    }

    private boolean removeTrack(int slot) {
        if (selectedPlaylist == null || slot >= rowKeys.size()) {
            return false;
        }
        player.removeFromPlaylist(selectedPlaylist, rowKeys.get(slot));
        return true;
    }

    private void renameSelected() {
        if (selectedPlaylist == null || nameBox == null) {
            return;
        }
        String name = nameBox.getText().trim();
        if (!name.isEmpty()) {
            player.renamePlaylist(selectedPlaylist, name);
        }
        Playlist pl = player.playlistById(selectedPlaylist);
        draftName = pl == null ? "" : pl.name();
    }

    private void deleteSelected() {
        if (selectedPlaylist == null) {
            return;
        }
        player.deletePlaylist(selectedPlaylist);
        selectedPlaylist = null;
        page = 0;
    }

    private void playSelected() {
        Playlist pl = selectedPlaylist == null ? null : player.playlistById(selectedPlaylist);
        if (pl != null) {
            player.playQueue(pl.id(), pl.tracks());
        }
    }

    /** Escape steps back one level: pick mode, then the playlist detail, then closes. */
    private boolean back() {
        if (pendingAddTrack != null) {
            pendingAddTrack = null;
            tab = Tab.LIBRARY;
            page = 0;
            return true;
        }
        if (selectedPlaylist != null) {
            selectedPlaylist = null;
            page = 0;
            return true;
        }
        close();
        return false;
    }

    private void close() {
        ClientConfig.getInstance().save();
        mc.displayGuiScreen(parent);
    }

    private void nudgeVolume(float delta) {
        float v = Math.round((player.volume() + delta) * 100F) / 100F;
        player.setVolume(Math.max(0F, Math.min(1F, v)));
    }

    @Override
    protected void mouseClicked(int mouseX, int mouseY, int mouseButton) {
        if (nameBox != null) {
            nameBox.mouseClicked(mouseX, mouseY, mouseButton);
        }
        if (mouseButton != 0) {
            return;
        }
        for (GuiButton button : new ArrayList<GuiButton>(buttonList)) {
            if (button.mousePressed(mc, mouseX, mouseY)) {
                button.playPressSound(mc.getSoundHandler());
                actionPerformed(button);
                return;
            }
        }
    }

    @Override
    protected void keyTyped(char typedChar, int keyCode) {
        if (keyCode == Keyboard.KEY_ESCAPE) {
            if (back()) {
                initGui();
            }
            return;
        }
        if (nameBox != null && nameBox.isFocused()) {
            if (keyCode == Keyboard.KEY_RETURN || keyCode == Keyboard.KEY_NUMPADENTER) {
                renameSelected();
                initGui();
                return;
            }
            nameBox.textboxKeyTyped(typedChar, keyCode);
            draftName = nameBox.getText();
        }
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        if (mc.theWorld == null) {
            MenuSkin.paintBackdrop(width, height);
        }
        SuitePainter.paintPanel(width, height);

        int headerY = SuiteTheme.PANEL_TOP + SuiteTheme.PAD + (SuiteTheme.HEADER_H - 8) / 2;
        String title = Lang.tr(pendingAddTrack != null ? "espectral.media.addto" : "espectral.media.title");
        Gfx.textGradient(title, innerX, headerY, SuiteTheme.accentGradientLeft(), SuiteTheme.accentGradientRight(), true);
        if (tab == Tab.LIBRARY) {
            Gfx.text("  " + player.library().tracks().size(), innerX + fontRendererObj.getStringWidth(title), headerY,
                    SuiteTheme.TEXT_DIM, false);
        }

        Gfx.text(fit(nowPlayingText(), innerW), innerX, NOW_Y, SuiteTheme.TEXT, false);
        String volume = Lang.tr("espectral.media.volume") + ": " + Math.round(player.volume() * 100F) + "%";
        Gfx.textCentered(fit(volume, volumeRight - volumeLeft), (volumeLeft + volumeRight) / 2, CONTROLS_Y + 5,
                SuiteTheme.TEXT, false);

        if (rowKeys.isEmpty()) {
            if (tab == Tab.LIBRARY) {
                String key = player.library().isScanning() ? "espectral.media.scanning" : "espectral.media.empty";
                Gfx.text(Lang.tr(key), innerX + 4, LIST_TOP + 6, SuiteTheme.TEXT_DIM, false);
            } else if (selectedPlaylist != null) {
                Gfx.text(Lang.tr("espectral.media.playlist.empty"), innerX + 4, LIST_TOP + SuiteTheme.ROW_H + 6,
                        SuiteTheme.TEXT_DIM, false);
            }
        }

        if (nameBox != null) {
            int boxW = innerW - RENAME_W - 4;
            Gfx.fill(innerX, LIST_TOP, innerX + boxW, LIST_TOP + FIELD_H, 0xA0000000);
            Gfx.outline(innerX, LIST_TOP, boxW, FIELD_H,
                    nameBox.isFocused() ? SuiteTheme.panelEdge() : SuiteTheme.accentHairline());
            nameBox.drawTextBox();
        }

        if (lastPages > 1) {
            Gfx.textCentered(Lang.tr("espectral.suite.page", page + 1, lastPages), innerX + innerW / 2, pagerY + 6,
                    SuiteTheme.TEXT_DIM, false);
        }
        Gfx.textCentered(fit(Lang.tr("espectral.media.credits"), innerW), innerX + innerW / 2, creditsY,
                SuiteTheme.TEXT_DIM, false);

        super.drawScreen(mouseX, mouseY, partialTicks);
        SuiteButton.drawTooltips(buttonList, mouseX, mouseY, width, height);
    }

    private String nowPlayingText() {
        MediaTrack cur = player.current();
        if (cur == null) {
            return Lang.tr("espectral.media.nowplaying.none");
        }
        String total = cur.durationMs() > 0 ? MediaToast.clock(cur.durationMs()) : "--:--";
        return Lang.tr("espectral.media.nowplaying") + ": " + cur.displayName() + "  ("
                + MediaToast.clock(player.elapsedMs()) + " / " + total + ")";
    }

    private String fit(String text, int width) {
        return fontRendererObj.trimStringToWidth(text, Math.max(0, width));
    }

    @Override
    public void onGuiClosed() {
        Keyboard.enableRepeatEvents(false);
    }

    @Override
    public boolean doesGuiPauseGame() {
        return true;
    }
}
