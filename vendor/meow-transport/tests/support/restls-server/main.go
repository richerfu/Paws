// restls-server harness: upstream restls server + local TLS cover.
//
//	-listen  restls listen address (e.g. 127.0.0.1:0)
//	-cover   cover TLS listen address (e.g. 127.0.0.1:0)
//	-password restls PSK
//	-script  restls record script (optional)
//
// Prints "LISTEN=<addr> COVER=<addr>" on stdout once both are up, then
// echoes authenticated restls payloads back to the client.
package main

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"flag"
	"fmt"
	"io"
	"math/big"
	"net"
	"os"
	"time"

	restls "github.com/metacubex/restls-client-go"
)

func selfSignedCert() (tls.Certificate, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return tls.Certificate{}, err
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
		return tls.Certificate{}, err
	}
	return tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}, nil
}

func main() {
	listen := flag.String("listen", "127.0.0.1:0", "restls listen addr")
	coverAddr := flag.String("cover", "127.0.0.1:0", "cover TLS listen addr")
	password := flag.String("password", "test-password", "restls PSK")
	script := flag.String("script", "", "restls record script")
	flag.Parse()

	cert, err := selfSignedCert()
	if err != nil {
		fmt.Fprintln(os.Stderr, "cert:", err)
		os.Exit(1)
	}
	coverLn, err := tls.Listen("tcp", *coverAddr, &tls.Config{
		Certificates: []tls.Certificate{cert},
		MinVersion:   tls.VersionTLS12,
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, "cover listen:", err)
		os.Exit(1)
	}
	// The cover only needs to complete handshakes; data is never sent by
	// an authenticated restls client. Drain and discard.
	go func() {
		for {
			c, err := coverLn.Accept()
			if err != nil {
				return
			}
			go func() {
				defer c.Close()
				io.Copy(io.Discard, c)
			}()
		}
	}()

	ln, err := net.Listen("tcp", *listen)
	if err != nil {
		fmt.Fprintln(os.Stderr, "listen:", err)
		os.Exit(1)
	}
	fmt.Printf("LISTEN=%s COVER=%s\n", ln.Addr(), coverLn.Addr())

	for {
		raw, err := ln.Accept()
		if err != nil {
			return
		}
		go func() {
			cfg := &restls.RestlsServerConfig{
				ServerHostname: coverLn.Addr().String(),
				Password:       *password,
				RestlsScript:   *script,
			}
			conn, err := restls.RestlsServer(context.Background(), raw, cfg)
			if err != nil {
				raw.Close()
				fmt.Fprintln(os.Stderr, "restls server:", err)
				return
			}
			defer conn.Close()
			io.Copy(conn, conn) // echo
		}()
	}
}
