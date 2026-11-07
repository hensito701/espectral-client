package es.spectral.menu.media;

/**
 * Fixed-size PCM ring buffer shared by the decode thread (writer) and the
 * game's sound-engine thread (reader).
 *
 * <p>Bounded by construction: the whole decoded audio never exists in memory,
 * only this buffer. Sized for the vanilla streaming queue (up to four
 * {@code Channel.BUFFER_DURATION_SECONDS} buffers ≈ 4 s of 44.1 kHz stereo)
 * plus decode-ahead slack, so the reader never underruns under normal load.
 *
 * <p>{@link #write} never blocks; {@link #read} never blocks. The decode thread
 * parks on this monitor when the ring is full and the reader wakes it after
 * consuming bytes, so the whole loop costs no allocation and no polling.
 */
final class MediaRing {

    /**
     * ≈ 4.5 s of 44.1 kHz 16-bit stereo — bounded and independent of file size.
     * Together with the stream's one-buffer scratch (176 400 B) and vanilla's
     * four queued OpenAL uploads the player's total extra heap stays under
     * ~1.7 MB.
     */
    static final int CAPACITY = 786_432;

    private final byte[] buffer = new byte[CAPACITY];
    private int head;
    private int tail;
    private int count;

    /** Copies as much of {@code src} as fits and returns the byte count (may be 0). */
    synchronized int write(byte[] src, int off, int len) {
        int room = CAPACITY - count;
        int n = Math.min(len, room);
        int first = Math.min(n, CAPACITY - tail);
        System.arraycopy(src, off, buffer, tail, first);
        if (n > first) {
            System.arraycopy(src, off + first, buffer, 0, n - first);
        }
        tail = (tail + n) % CAPACITY;
        count += n;
        notifyAll();
        return n;
    }

    /** Copies up to {@code len} buffered bytes into {@code dst}; returns 0 when empty. */
    synchronized int read(byte[] dst, int off, int len) {
        int n = Math.min(len, count);
        if (n <= 0) {
            return 0;
        }
        int first = Math.min(n, CAPACITY - head);
        System.arraycopy(buffer, head, dst, off, first);
        if (n > first) {
            System.arraycopy(buffer, 0, dst, off + first, n - first);
        }
        head = (head + n) % CAPACITY;
        count -= n;
        notifyAll();
        return n;
    }

    /** Waits up to {@code millis} for the ring to have room; returns room after the wait. */
    synchronized int awaitRoom(int millis) throws InterruptedException {
        if (CAPACITY - count <= 0) {
            wait(millis);
        }
        return CAPACITY - count;
    }

    synchronized int available() {
        return count;
    }

    synchronized void clear() {
        head = 0;
        tail = 0;
        count = 0;
        notifyAll();
    }
}
