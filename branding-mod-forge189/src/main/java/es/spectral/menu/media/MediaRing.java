package es.spectral.menu.media;

/**
 * Fixed-size PCM ring buffer shared by the decode thread (writer) and the
 * audio thread (reader).
 *
 * <p>Bounded by construction: the whole decoded audio never exists in memory,
 * only this buffer. {@link #write} and {@link #read} never block; the decode
 * thread parks on this monitor when the ring is full and the reader wakes it
 * after consuming bytes, so the loop costs no allocation and no polling.
 */
final class MediaRing {

    /** 4.5 s of 44.1 kHz 16-bit stereo: matches the shipped Fabric build; bounded regardless of file size. */
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

    /** Waits up to {@code millis} for bytes to arrive; returns the byte count after the wait. */
    synchronized int awaitData(int millis) throws InterruptedException {
        if (count <= 0) {
            wait(millis);
        }
        return count;
    }

    /** Bytes currently buffered. */
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
