// Package httpobfs implements the SIP003 HTTP byte-stream envelope. It is
// independent of TUN, routing and Shadowsocks encryption.
package httpobfs

import (
	"bufio"
	"bytes"
	"crypto/rand"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"sync"
)

const MaxResponseHeader = 16 * 1024

type Conn struct {
	net.Conn
	reader                *bufio.Reader
	host, port            string
	readMu, writeMu       sync.Mutex
	firstRead, firstWrite bool
	readErr, writeErr     error
}

func New(conn net.Conn, host, port string) (net.Conn, error) {
	if conn == nil || host == "" || strings.ContainsAny(host, "\r\n") || strings.ContainsAny(port, "\r\n") {
		return nil, errors.New("invalid HTTP-obfs endpoint")
	}
	return &Conn{Conn: conn, reader: bufio.NewReaderSize(conn, 4096), host: host, port: port, firstRead: true, firstWrite: true}, nil
}

func (c *Conn) Read(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	c.readMu.Lock()
	defer c.readMu.Unlock()
	if c.readErr != nil {
		return 0, c.readErr
	}
	if c.firstRead {
		const terminator = "\r\n\r\n"
		matched := 0
		for count := 0; count < MaxResponseHeader; count++ {
			b, err := c.reader.ReadByte()
			if err != nil {
				if errors.Is(err, io.EOF) && count > 0 {
					err = io.ErrUnexpectedEOF
				}
				c.readErr = err
				return 0, err
			}
			if b == terminator[matched] {
				matched++
			} else if b == '\r' {
				matched = 1
			} else {
				matched = 0
			}
			if matched == len(terminator) {
				c.firstRead = false
				break
			}
		}
		if c.firstRead {
			c.readErr = errors.New("HTTP-obfs response header exceeds limit")
			return 0, c.readErr
		}
	}
	// A header-only first packet is not EOF. Wait for encrypted response bytes.
	return c.reader.Read(p)
}

func (c *Conn) Write(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	if c.writeErr != nil {
		return 0, c.writeErr
	}
	if !c.firstWrite {
		return c.Conn.Write(p)
	}
	var random [16]byte
	if _, err := rand.Read(random[:]); err != nil {
		return 0, err
	}
	req, err := http.NewRequest("GET", "http://"+c.host+"/", bytes.NewReader(p))
	if err != nil {
		return 0, err
	}
	req.Host = c.host
	if c.port != "80" {
		req.Host = fmt.Sprintf("%s:%s", c.host, c.port)
	}
	req.Header.Set("User-Agent", fmt.Sprintf("curl/7.%d.%d", int(random[0])%54, int(random[1])%2))
	req.Header.Set("Connection", "Upgrade")
	req.Header.Set("Upgrade", "websocket")
	req.Header.Set("Sec-WebSocket-Key", base64.URLEncoding.EncodeToString(random[:]))
	req.ContentLength = int64(len(p))
	var wire bytes.Buffer
	if err = req.Write(&wire); err != nil {
		return 0, err
	}
	c.firstWrite = false
	for wire.Len() > 0 {
		n, e := c.Conn.Write(wire.Bytes())
		if n < 0 || n > wire.Len() {
			e = io.ErrShortWrite
			n = 0
		}
		wire.Next(n)
		if e != nil {
			c.writeErr = e
			return 0, e
		}
		if n == 0 {
			c.writeErr = io.ErrShortWrite
			return 0, c.writeErr
		}
	}
	return len(p), nil
}
