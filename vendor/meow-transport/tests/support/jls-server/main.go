// jls-server harness: upstream jls-tls server (metacubex/jls-tls).
//
//	-listen   jls listen address (e.g. 127.0.0.1:0)
//	-username jls username (user_iv)
//	-password jls password (user_pwd)
//
// Prints "LISTEN=<addr>" on stdout once up, then echoes authenticated
// jls payloads back to the client. A failed-auth connection is relayed
// by the upstream server to the camouflage site — nothing is dialed, so
// those connections simply die, which is what the tests assert.
package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"flag"
	"fmt"
	"io"
	"math/big"
	"net"
	"os"
	"time"

	jlstls "github.com/metacubex/jls-tls"
)

func selfSignedCert() (jlstls.Certificate, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return jlstls.Certificate{}, err
	}
	tpl := &x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject:      pkix.Name{CommonName: "cover.example.com"},
		DNSNames:     []string{"cover.example.com", "localhost"},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(24 * time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
		IsCA:                  true,
	}
	der, err := x509.CreateCertificate(rand.Reader, tpl, tpl, &key.PublicKey, key)
	if err != nil {
		return jlstls.Certificate{}, err
	}
	return jlstls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}, nil
}

func main() {
	listen := flag.String("listen", "127.0.0.1:0", "jls listen addr")
	username := flag.String("username", "alice", "jls username")
	password := flag.String("password", "test-password", "jls password")
	flag.Parse()

	cert, err := selfSignedCert()
	if err != nil {
		fmt.Fprintln(os.Stderr, "cert:", err)
		os.Exit(1)
	}

	ln, err := net.Listen("tcp", *listen)
	if err != nil {
		fmt.Fprintln(os.Stderr, "listen:", err)
		os.Exit(1)
	}
	fmt.Printf("LISTEN=%s\n", ln.Addr())

	for {
		raw, err := ln.Accept()
		if err != nil {
			return
		}
		go func() {
			conn := jlstls.Server(raw, &jlstls.Config{
				Certificates: []jlstls.Certificate{cert},
				MinVersion:   jlstls.VersionTLS13,
				JLSConfig: &jlstls.JLSConfig{
					Enable: true,
					Users: []jlstls.JLSUser{{
						Username: *username,
						Password: *password,
					}},
				},
			})
			if err := conn.Handshake(); err != nil {
				raw.Close()
				fmt.Fprintln(os.Stderr, "jls handshake:", err)
				return
			}
			defer conn.Close()
			io.Copy(conn, conn) // echo
		}()
	}
}
