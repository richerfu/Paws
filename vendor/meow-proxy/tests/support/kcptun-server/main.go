// kcptun-server harness: the real upstream server side for the in-process
// kcptun SIP003 plugin — kcp-go (crypt+FEC envelope) → optional snappy
// compStream → xtaci/smux v1 → go-shadowsocks2 termination.
//
//	-listen    UDP listen address (e.g. 127.0.0.1:0)
//	-key       kcptun key (pbkdf2'd with the "kcp-go" salt)
//	-crypt     crypt name (aes, aes-128-gcm, salsa20, none, xor, ...)
//	-nocomp    disable the snappy stream layer
//	-ds/-ps    FEC data/parity shards
//	-sscipher/-sspass  shadowsocks layer termination
//
// Prints "LISTEN=<addr>" once up. Every accepted smux stream is an SS
// connection: the harness reads the target address off the decrypted
// stream and echoes the payload back.
package main

import (
	"crypto/sha1"
	"encoding/binary"
	"flag"
	"fmt"
	"io"
	"net"
	"os"

	"github.com/golang/snappy"
	kcp "github.com/xtaci/kcp-go/v5"
	"github.com/xtaci/smux"
	"golang.org/x/crypto/pbkdf2"

	"github.com/shadowsocks/go-shadowsocks2/core"
	"github.com/shadowsocks/go-shadowsocks2/socks"
)

// compStream mirrors kcptun's NewCompStream: snappy framed I/O over the
// KCP session, flushing on every write.
type compStream struct {
	net.Conn
	w *snappy.Writer
	r *snappy.Reader
}

func newCompStream(conn net.Conn) *compStream {
	return &compStream{
		Conn: conn,
		w:    snappy.NewBufferedWriter(conn),
		r:    snappy.NewReader(conn),
	}
}

func (c *compStream) Read(b []byte) (int, error)  { return c.r.Read(b) }
func (c *compStream) Write(b []byte) (int, error) {
	n, err := c.w.Write(b)
	if err == nil {
		err = c.w.Flush()
	}
	return n, err
}

func blockCrypt(crypt, key string) (kcp.BlockCrypt, error) {
	pass := pbkdf2.Key([]byte(key), []byte("kcp-go"), 4096, 32, sha1.New)
	switch crypt {
	case "sm4":
		return kcp.NewSM4BlockCrypt(pass[:16])
	case "aes", "aes-256":
		return kcp.NewAESBlockCrypt(pass)
	case "aes-128":
		return kcp.NewAESBlockCrypt(pass[:16])
	case "aes-192":
		return kcp.NewAESBlockCrypt(pass[:24])
	case "aes-128-gcm":
		return kcp.NewAESGCMCrypt(pass[:16])
	case "salsa20":
		return kcp.NewSalsa20BlockCrypt(pass)
	case "blowfish":
		return kcp.NewBlowfishBlockCrypt(pass)
	case "twofish":
		return kcp.NewTwofishBlockCrypt(pass)
	case "cast5":
		return kcp.NewCast5BlockCrypt(pass[:16])
	case "3des":
		return kcp.NewTripleDESBlockCrypt(pass[:24])
	case "xtea":
		return kcp.NewXTEABlockCrypt(pass[:16])
	case "tea":
		return kcp.NewTEABlockCrypt(pass[:16])
	case "xor":
		return kcp.NewSimpleXORBlockCrypt(pass)
	case "none":
		return kcp.NewNoneBlockCrypt(pass)
	case "null":
		return nil, nil
	default:
		return nil, fmt.Errorf("unknown crypt %q", crypt)
	}
}

func main() {
	listen := flag.String("listen", "127.0.0.1:0", "kcp listen addr")
	key := flag.String("key", "it's a secrect", "kcptun key")
	crypt := flag.String("crypt", "aes", "crypt name")
	nocomp := flag.Bool("nocomp", false, "disable snappy")
	ds := flag.Int("ds", 10, "fec data shards")
	ps := flag.Int("ps", 3, "fec parity shards")
	sscipher := flag.String("sscipher", "aes-256-gcm", "ss cipher")
	sspass := flag.String("sspass", "test-password", "ss password")
	raw := flag.Bool("raw", false, "echo raw KCP bytes (no smux/snappy/ss)")
	rawsnappy := flag.Bool("rawsnappy", false, "echo raw bytes through the snappy layer only (no smux/ss)")
	rawsmux := flag.Bool("rawsmux", false, "echo smux streams without the SS layer")
	flag.Parse()

	block, err := blockCrypt(*crypt, *key)
	if err != nil {
		fmt.Fprintln(os.Stderr, "block:", err)
		os.Exit(1)
	}
	l, err := kcp.ListenWithOptions(*listen, block, *ds, *ps)
	if err != nil {
		fmt.Fprintln(os.Stderr, "listen:", err)
		os.Exit(1)
	}
	fmt.Printf("LISTEN=%s\n", l.Addr())

	ciph, err := core.PickCipher(*sscipher, nil, *sspass)
	if err != nil {
		fmt.Fprintln(os.Stderr, "cipher:", err)
		os.Exit(1)
	}

	for {
		sess, err := l.AcceptKCP()
		if err != nil {
			fmt.Fprintln(os.Stderr, "accept:", err)
			continue
		}
		if *raw {
			go func() { defer sess.Close(); io.Copy(sess, sess) }()
			continue
		}
		if *rawsnappy {
			go func() {
				c := newCompStream(sess)
				defer sess.Close()
				io.Copy(c, c)
			}()
			continue
		}
		go func() {
			var conn net.Conn = sess
			if !*nocomp {
				conn = newCompStream(sess)
			}
			mux, err := smux.Server(conn, nil)
			if err != nil {
				sess.Close()
				return
			}
			defer mux.Close()
			for {
				stream, err := mux.AcceptStream()
				if err != nil {
					return
				}
				if *rawsmux {
					go func() { defer stream.Close(); io.Copy(stream, stream) }()
				} else {
					go handle(stream, ciph)
				}
			}
		}()
	}
}

func handle(stream net.Conn, ciph core.Cipher) {
	defer stream.Close()
	sc := ciph.StreamConn(stream)
	tgt, err := socks.ReadAddr(sc)
	if err != nil {
		fmt.Fprintln(os.Stderr, "readaddr:", err)
		return
	}
	// Legacy UDP-over-TCP: the SS target is the UoT magic address and the
	// stream then carries `uotAddr ‖ u16be len ‖ payload` datagrams.
	if tgt.String() == "sp.udp-over-tcp.arpa:0" {
		uotLoop(sc)
		return
	}
	io.Copy(sc, sc)
}

// uotLoop echoes `uot addr ‖ len ‖ payload` frames back verbatim — the
// client's UDP relay round-trips through them.
func uotLoop(c net.Conn) {
	for {
		addr, err := readUotAddr(c)
		if err != nil {
			return
		}
		var lb [2]byte
		if _, err := io.ReadFull(c, lb[:]); err != nil {
			return
		}
		payload := make([]byte, binary.BigEndian.Uint16(lb[:]))
		if _, err := io.ReadFull(c, payload); err != nil {
			return
		}
		var out []byte
		out = append(out, addr...)
		out = append(out, lb[:]...)
		out = append(out, payload...)
		if _, err := c.Write(out); err != nil {
			return
		}
	}
}

// readUotAddr parses the uot.AddrParser family bytes (0=v4, 1=v6,
// 2=len+domain) + u16be port and returns the raw header bytes for echo.
func readUotAddr(r io.Reader) ([]byte, error) {
	var atyp [1]byte
	if _, err := io.ReadFull(r, atyp[:]); err != nil {
		return nil, err
	}
	var body []byte
	switch atyp[0] {
	case 0:
		body = make([]byte, 4)
	case 1:
		body = make([]byte, 16)
	case 2:
		var l [1]byte
		if _, err := io.ReadFull(r, l[:]); err != nil {
			return nil, err
		}
		body = append([]byte{l[0]}, make([]byte, int(l[0]))...)
		if _, err := io.ReadFull(r, body[1:]); err != nil {
			return nil, err
		}
	default:
		return nil, fmt.Errorf("bad uot atyp %d", atyp[0])
	}
	if atyp[0] != 2 {
		if _, err := io.ReadFull(r, body); err != nil {
			return nil, err
		}
	}
	var port [2]byte
	if _, err := io.ReadFull(r, port[:]); err != nil {
		return nil, err
	}
	out := append([]byte{atyp[0]}, body...)
	return append(out, port[:]...), nil
}
