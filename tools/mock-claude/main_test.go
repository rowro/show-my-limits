package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestQuotaScenarios(t *testing.T) {
	now := time.Unix(1800000000, 0)
	for _, tc := range []struct{ name, five, seven, status string }{
		{"normal", "0.35", "0.42", "allowed"},
		{"low", "0.92", "0.95", "allowed"},
		{"blocked", "1.00", "1.00", "rejected"},
		{"weekly-blocked", "0.25", "1.00", "rejected"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			h, err := quotaHeaders(tc.name, 0, now)
			if err != nil {
				t.Fatal(err)
			}
			if h.Get(prefix+"5h-Utilization") != tc.five || h.Get(prefix+"7d-Utilization") != tc.seven || h.Get(prefix+"Status") != tc.status {
				t.Fatal(h)
			}
			if h.Get(prefix+"5h-Reset") != "1800005400" || h.Get(prefix+"7d-Reset") != "1800259200" {
				t.Fatal("reset times drifted", h)
			}
		})
	}
	h, _ := quotaHeaders("progress", 200, now)
	if h.Get(prefix+"5h-Utilization") != "1.00" || h.Get(prefix+"7d-Utilization") != "1.00" || h.Get(prefix+"Status") != "rejected" {
		t.Fatal(h)
	}
	h, _ = quotaHeaders("missing", 0, now)
	if len(h) != 0 {
		t.Fatal(h)
	}
	if _, err := quotaHeaders("typo", 0, now); err == nil {
		t.Fatal("unknown scenario accepted")
	}
}

func TestRequiresHTTPSWithoutCredentials(t *testing.T) {
	for _, input := range []string{"http://localhost/response-headers", "https://user:secret@example.com/response-headers", "https:///missing-host", "https://example.com/#fragment", "https://example.com/?query=value"} {
		if _, err := requestURL(input, nil); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
}

func TestRealTLSHeaderRoundTrip(t *testing.T) {
	want, _ := quotaHeaders("weekly-blocked", 0, time.Now())
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "" || r.Header.Get("Cookie") != "" {
			t.Error("credentials sent")
		}
		if r.Method != http.MethodGet {
			t.Error("unexpected method")
		}
		for k, v := range r.URL.Query() {
			for _, value := range v {
				w.Header().Add(k, value)
			}
		}
		io.WriteString(w, "synthetic response")
	}))
	defer server.Close()
	target, err := requestURL(server.URL+"/response-headers", want)
	if err != nil {
		t.Fatal(err)
	}
	// Use a real, explicitly trusted test CA. No InsecureSkipVerify in tests or client.
	roots := x509.NewCertPool()
	roots.AddCert(server.Certificate())
	trusted := &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}}}
	defer trusted.CloseIdleConnections()
	var out bytes.Buffer
	if err := send(context.Background(), trusted, target, want, &out); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), "1.00") {
		t.Fatal(out.String())
	}
	untrusted, err := newClient("", false)
	if err != nil {
		t.Fatal(err)
	}
	defer untrusted.CloseIdleConnections()
	if err := send(context.Background(), untrusted, target, want, io.Discard); err == nil {
		t.Fatal("untrusted server certificate accepted")
	}
}

func TestRejectsMissingEchoAndRedirects(t *testing.T) {
	want, _ := quotaHeaders("normal", 0, time.Now())
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/redirect" {
			http.Redirect(w, r, "/", http.StatusFound)
			return
		}
		io.WriteString(w, "no headers")
	}))
	defer server.Close()
	trusted := server.Client()
	if err := send(context.Background(), trusted, server.URL, want, io.Discard); err == nil {
		t.Fatal("missing echo accepted")
	}
	client, _ := newClient("", false)
	trusted.CheckRedirect = client.CheckRedirect
	if err := send(context.Background(), trusted, server.URL+"/redirect", nil, io.Discard); err == nil {
		t.Fatal("redirect accepted")
	}
	if _, err := newClient("nonexistent-certificate.pem", false); err == nil {
		t.Fatal("missing CA silently ignored")
	}
}

func TestIPv4ModeDoesNotDialIPv6(t *testing.T) {
	client, err := newClient("", true)
	if err != nil {
		t.Fatal(err)
	}
	defer client.CloseIdleConnections()
	transport := client.Transport.(*http.Transport)
	conn, err := transport.DialContext(context.Background(), "tcp", "[::1]:443")
	if conn != nil {
		conn.Close()
	}
	if err == nil {
		t.Fatal("IPv4-only mode dialed an IPv6 address")
	}
}
