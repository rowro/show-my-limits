// Command claude generates synthetic HTTPS quota responses for capture testing.
package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"flag"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"os"
	"os/signal"
	"strings"
	"time"
)

const prefix = "Anthropic-Ratelimit-Unified-"
const defaultEndpoint = "https://httpbingo.org/response-headers"

type scenario struct {
	five, seven float64
	missing     bool
}

var scenarios = map[string]scenario{
	"normal":         {five: .35, seven: .42},
	"low":            {five: .92, seven: .95},
	"blocked":        {five: 1, seven: 1},
	"weekly-blocked": {five: .25, seven: 1},
	"progress":       {five: .35, seven: .42},
	"missing":        {missing: true},
}

func quotaHeaders(name string, index int, started time.Time) (http.Header, error) {
	s, ok := scenarios[name]
	if !ok {
		return nil, fmt.Errorf("unknown scenario %q", name)
	}
	h := make(http.Header)
	if s.missing {
		return h, nil
	}
	if name == "progress" {
		s.five = math.Min(1, s.five+float64(index)*.05)
		s.seven = math.Min(1, s.seven+float64(index)*.01)
	}
	status := "allowed"
	for _, w := range []struct {
		name  string
		used  float64
		reset time.Time
	}{
		{"5h", s.five, started.Add(90 * time.Minute)},
		{"7d", s.seven, started.Add(72 * time.Hour)},
	} {
		state := "allowed"
		if w.used >= 1 {
			state = "rejected"
			status = "rejected"
		}
		h.Set(prefix+w.name+"-Utilization", fmt.Sprintf("%.2f", w.used))
		h.Set(prefix+w.name+"-Reset", fmt.Sprint(w.reset.Unix()))
		h.Set(prefix+w.name+"-Status", state)
	}
	h.Set(prefix+"Status", status)
	return h, nil
}

func requestURL(endpoint string, headers http.Header) (string, error) {
	u, err := url.Parse(endpoint)
	if err != nil || u.Scheme != "https" || u.Hostname() == "" || u.User != nil || u.Fragment != "" || u.RawQuery != "" {
		return "", errors.New("endpoint must be an HTTPS response-headers URL without credentials, query or fragment")
	}
	q := url.Values{}
	for name, values := range headers {
		for _, value := range values {
			q.Add(name, value)
		}
	}
	u.RawQuery = q.Encode()
	return u.String(), nil
}

func newClient(caFile string) (*http.Client, error) {
	config := &tls.Config{MinVersion: tls.VersionTLS12}
	if caFile != "" {
		roots, err := x509.SystemCertPool()
		if err != nil {
			return nil, fmt.Errorf("read system trust: %w", err)
		}
		pem, err := os.ReadFile(caFile)
		if err != nil {
			return nil, fmt.Errorf("read extra CA: %w", err)
		}
		if !roots.AppendCertsFromPEM(pem) {
			return nil, errors.New("extra CA file contains no PEM certificates")
		}
		config.RootCAs = roots
	}
	return &http.Client{
		// Use the real OS trust store and direct TCP/TLS. Environment proxy settings
		// would bypass the process-capture path this tool is supposed to exercise.
		Transport: &http.Transport{TLSClientConfig: config, DisableKeepAlives: true, ForceAttemptHTTP2: false},
		Timeout:   20 * time.Second,
		CheckRedirect: func(_ *http.Request, _ []*http.Request) error {
			return errors.New("unexpected redirect from test server")
		},
	}, nil
}

func send(ctx context.Context, client *http.Client, target string, expected http.Header, out io.Writer) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, target, nil)
	if err != nil {
		return err
	}
	req.Header.Set("User-Agent", "Show-My-Limits-Mock/1")
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("test server returned HTTP %d", resp.StatusCode)
	}
	for name, values := range expected {
		if resp.Header.Get(name) != values[0] {
			return fmt.Errorf("server did not echo %s correctly", name)
		}
	}
	if len(expected) == 0 {
		for name := range resp.Header {
			if strings.HasPrefix(strings.ToLower(name), strings.ToLower(prefix)) {
				return fmt.Errorf("unexpected quota header %s in missing scenario", name)
			}
		}
	}
	if resp.TLS == nil || len(resp.TLS.VerifiedChains) == 0 {
		return errors.New("HTTPS connection was not verified")
	}
	fmt.Fprintf(out, "HTTP %d | TLS verified | issuer: %s\n", resp.StatusCode, resp.TLS.PeerCertificates[0].Issuer.CommonName)
	for _, window := range []string{"5h", "7d"} {
		if value := resp.Header.Get(prefix + window + "-Utilization"); value != "" {
			fmt.Fprintf(out, "  %s used: %s | reset: %s | %s\n", window, value, resp.Header.Get(prefix+window+"-Reset"), resp.Header.Get(prefix+window+"-Status"))
		}
	}
	if len(expected) == 0 {
		fmt.Fprintln(out, "  No quota headers (intentional).")
	}
	_, err = io.Copy(io.Discard, io.LimitReader(resp.Body, 64*1024))
	return err
}

func run() error {
	name := flag.String("scenario", "normal", "normal, low, blocked, weekly-blocked, progress, missing")
	count := flag.Int("count", 0, "requests to send; 0 repeats until Ctrl+C")
	interval := flag.Duration("interval", 5*time.Second, "delay between requests (at least 1s)")
	endpoint := flag.String("endpoint", defaultEndpoint, "HTTPS response-headers endpoint")
	caFile := flag.String("ca", "", "optional extra public CA PEM; default uses OS trust only")
	flag.Parse()
	if flag.NArg() != 0 || *count < 0 || *interval < time.Second {
		return errors.New("invalid arguments: count must be >= 0, interval >= 1s; use --help")
	}
	started := time.Now()
	first, err := quotaHeaders(*name, 0, started)
	if err != nil {
		return err
	}
	target, err := requestURL(*endpoint, first)
	if err != nil {
		return err
	}
	u, _ := url.Parse(target)
	fmt.Printf("Show My Limits — synthetic Claude client (PID %d)\n", os.Getpid())
	fmt.Printf("Server in Show My Limits: %s\nScenario: %s | Ctrl+C to stop\n", u.Hostname(), *name)
	fmt.Println("No Claude account, messages or tokens are used. Only synthetic quota values are sent.")
	if *caFile != "" {
		fmt.Println("Extra CA supplied: this run does not prove that OS trust was configured correctly.")
	}
	client, err := newClient(*caFile)
	if err != nil {
		return err
	}
	defer client.CloseIdleConnections()
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	for i := 0; *count == 0 || i < *count; i++ {
		headers, _ := quotaHeaders(*name, i, started)
		target, _ := requestURL(*endpoint, headers)
		fmt.Printf("\n[%s] Request %d\n", time.Now().Format("15:04:05"), i+1)
		if err := send(ctx, client, target, headers, os.Stdout); err != nil {
			if ctx.Err() != nil {
				return nil
			}
			return fmt.Errorf("request failed: %w\nCheck CA trust and capture settings; TLS verification is never disabled", err)
		}
		if *count != 0 && i+1 == *count {
			break
		}
		timer := time.NewTimer(*interval)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil
		case <-timer.C:
		}
	}
	fmt.Println("\nRequests completed. Confirm capture in Show My Limits; an HTTPS response alone does not prove interception.")
	return nil
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
