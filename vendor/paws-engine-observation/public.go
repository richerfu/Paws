package main

import (
	"bytes"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"path"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

type pawsNetworkServices struct {
	Ports struct {
		MixedPort         uint16 `json:"mixedPort" yaml:"mixedPort"`
		ControllerPort    uint16 `json:"controllerPort" yaml:"controllerPort"`
		MixedEnabled      bool   `json:"mixedEnabled" yaml:"mixedEnabled"`
		ControllerEnabled bool   `json:"controllerEnabled" yaml:"controllerEnabled"`
	} `json:"ports" yaml:"ports"`
	Access struct {
		AllowLAN bool    `json:"allowLan" yaml:"allowLan"`
		Secret   *string `json:"secret" yaml:"secret"`
	} `json:"access" yaml:"access"`
}
type pawsPublicServer struct {
	address     string
	server      *http.Server
	active      atomic.Bool
	mu          sync.Mutex
	connections map[net.Conn]struct{}
	transport   *http.Transport
}
type pawsPublicListener struct {
	net.Listener
	owner *pawsPublicServer
}
type pawsPublicConnection struct {
	net.Conn
	owner *pawsPublicServer
	once  sync.Once
}

func (l pawsPublicListener) Accept() (net.Conn, error) {
	connection, err := l.Listener.Accept()
	if err != nil {
		return nil, err
	}
	tracked := &pawsPublicConnection{Conn: connection, owner: l.owner}
	l.owner.mu.Lock()
	if !l.owner.active.Load() {
		l.owner.mu.Unlock()
		_ = connection.Close()
		return nil, net.ErrClosed
	}
	l.owner.connections[tracked] = struct{}{}
	l.owner.mu.Unlock()
	return tracked, nil
}
func (c *pawsPublicConnection) Close() error {
	c.once.Do(func() { c.owner.mu.Lock(); delete(c.owner.connections, c); c.owner.mu.Unlock() })
	return c.Conn.Close()
}
func startPawsPublicServer(services pawsNetworkServices, privateAddress, privateSecret string) (*pawsPublicServer, error) {
	if !services.Ports.ControllerEnabled {
		return nil, nil
	}
	if services.Ports.ControllerPort < 1024 {
		return nil, errors.New("public controller port must be between 1024 and 65535")
	}
	target, err := url.Parse("http://" + privateAddress)
	if err != nil {
		return nil, errors.New("invalid private controller address")
	}
	host, _, err := net.SplitHostPort(privateAddress)
	if err != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() || privateSecret == "" {
		return nil, errors.New("private controller must use authenticated loopback")
	}
	listenHost := "127.0.0.1"
	if services.Access.AllowLAN {
		listenHost = "0.0.0.0"
	}
	listener, err := net.Listen("tcp", net.JoinHostPort(listenHost, strconv.Itoa(int(services.Ports.ControllerPort))))
	if err != nil {
		return nil, errors.New("public controller bind failed: " + err.Error())
	}
	owner := &pawsPublicServer{address: listener.Addr().String(), connections: make(map[net.Conn]struct{})}
	owner.active.Store(true)
	owner.transport = &http.Transport{Proxy: nil, DialContext: (&net.Dialer{Timeout: 2 * time.Second}).DialContext, ResponseHeaderTimeout: 30 * time.Second}
	proxy := &httputil.ReverseProxy{
		Rewrite: func(request *httputil.ProxyRequest) {
			request.SetURL(target)
			request.Out.Header.Set("Authorization", "Bearer "+privateSecret)
			if request.Out.URL.Path == "/configs" || request.Out.URL.Path == "/configs/" {
				request.Out.Header.Set("Accept-Encoding", "identity")
			}
			request.Out.Header.Del("Proxy-Authorization")
			request.Out.Header.Del("Cookie")
			query := request.Out.URL.Query()
			query.Del("token")
			request.Out.URL.RawQuery = query.Encode()
		},
		Transport: owner.transport,
		ModifyResponse: func(response *http.Response) error {
			if response.Request.URL.Path != "/configs" && response.Request.URL.Path != "/configs/" {
				return nil
			}
			defer response.Body.Close()
			var native map[string]json.RawMessage
			if err := json.NewDecoder(io.LimitReader(response.Body, 64*1024)).Decode(&native); err != nil {
				return errors.New("invalid engine config response")
			}
			public := make(map[string]any)
			for _, key := range []string{"mode", "log-level", "ipv6"} {
				if value, ok := native[key]; ok {
					public[key] = value
				}
			}
			public["mixed-port"] = 0
			if services.Ports.MixedEnabled {
				public["mixed-port"] = services.Ports.MixedPort
			}
			public["allow-lan"] = services.Access.AllowLAN
			public["bind-address"] = listenHost
			public["external-controller"] = net.JoinHostPort(listenHost, strconv.Itoa(int(services.Ports.ControllerPort)))
			encoded, err := json.Marshal(public)
			if err != nil {
				return err
			}
			response.Body = io.NopCloser(bytes.NewReader(encoded))
			response.ContentLength = int64(len(encoded))
			response.Header.Set("Content-Length", strconv.Itoa(len(encoded)))
			return nil
		},
		ErrorHandler: func(w http.ResponseWriter, _ *http.Request, _ error) {
			http.Error(w, "engine controller unavailable", http.StatusBadGateway)
		},
	}
	owner.server = &http.Server{ReadHeaderTimeout: 5 * time.Second, IdleTimeout: 60 * time.Second, ConnState: func(connection net.Conn, state http.ConnState) {
		owner.mu.Lock()
		defer owner.mu.Unlock()
		if state == http.StateNew {
			owner.connections[connection] = struct{}{}
		}
		if state == http.StateClosed {
			delete(owner.connections, connection)
		}
		// Retain StateHijacked until owner close; net/http does not close WS
		// connections as part of Server.Close/Shutdown.
	}, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !owner.active.Load() {
			http.Error(w, "engine session stopped", http.StatusServiceUnavailable)
			return
		}
		if !pawsPublicPathAllowed(r) {
			http.Error(w, "endpoint is private or unavailable", http.StatusForbidden)
			return
		}
		if services.Access.Secret != nil && *services.Access.Secret != "" {
			authorization := r.Header.Get("Authorization")
			token := ""
			if strings.HasPrefix(authorization, "Bearer ") {
				token = strings.TrimPrefix(authorization, "Bearer ")
			}
			if authorization == "" {
				token = r.URL.Query().Get("token")
			}
			if subtle.ConstantTimeCompare([]byte(token), []byte(*services.Access.Secret)) != 1 {
				http.Error(w, "unauthorized", http.StatusUnauthorized)
				return
			}
		}
		proxy.ServeHTTP(w, r)
	})}
	go func() { _ = owner.server.Serve(pawsPublicListener{listener, owner}) }()
	return owner, nil
}
func pawsPublicPathAllowed(r *http.Request) bool {
	p := r.URL.Path
	// Backends differ in escaped-segment routing. Reject ambiguous traversal
	// before forwarding, rather than relying on either router's normalization.
	for pass := 0; pass < 4; pass++ {
		if p == "" || !strings.HasPrefix(p, "/") || path.Clean(p) != p || strings.ContainsAny(p, "\\\x00") || strings.Contains(p, "//") {
			return false
		}
		decoded, err := url.PathUnescape(p)
		if err != nil {
			return false
		}
		if decoded == p {
			break
		}
		p = decoded
		if pass == 3 {
			return false
		}
	}
	parts := strings.Split(strings.TrimPrefix(p, "/"), "/")
	if len(parts) == 0 {
		return false
	}
	switch parts[0] {
	case "", "version", "proxies", "rules", "connections", "providers", "traffic", "memory", "logs", "dns", "cache", "group":
		return true
	case "configs":
		// Read responses are reduced to the public effective view; native
		// mutations would bypass application revision/owner transactions.
		return r.Method == http.MethodGet && (p == "/configs" || p == "/configs/")
	default:
		return false
	}
}
func (p *pawsPublicServer) close() error {
	if p == nil {
		return nil
	}
	p.active.Store(false)
	// Never wait indefinitely for /logs, /traffic, or an upgraded socket.
	p.mu.Lock()
	connections := make([]net.Conn, 0, len(p.connections))
	for connection := range p.connections {
		connections = append(connections, connection)
	}
	p.connections = make(map[net.Conn]struct{})
	p.mu.Unlock()
	for _, connection := range connections {
		_ = connection.Close()
	}
	p.transport.CloseIdleConnections()
	return p.server.Close()
}
