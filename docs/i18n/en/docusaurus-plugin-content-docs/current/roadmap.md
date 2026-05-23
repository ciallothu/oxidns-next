---
title: Roadmap
sidebar_position: 5
---

# Roadmap

The following outlines OxiDNS's planned development directions in delivery order.

```mermaid
flowchart LR
  A["① Custom Builds"] --> B["② IP Optimization"]
  B --> C["③ MikroTik Integration"]
  C --> D["④ OpenWrt Support"]
  D --> E["⑤ WebUI & Metrics"]

  style A fill:#f0fdfa,stroke:#0f766e,stroke-width:2px,color:#0f4c46
  style B fill:#f0fdfa,stroke:#0f766e,stroke-width:2px,color:#0f4c46
  style C fill:#f0fdfa,stroke:#0f766e,stroke-width:2px,color:#0f4c46
  style D fill:#f0fdfa,stroke:#0f766e,stroke-width:2px,color:#0f4c46
  style E fill:#f0fdfa,stroke:#0f766e,stroke-width:2px,color:#0f4c46
```

---

## Phase 1 · Custom Builds

Split compilation by plugin module so users can fork the repository, select only the plugins they need, produce a lean custom build, and keep it up to date via a configurable upgrade repository.

---

## Phase 2 · IP Optimization

Test multiple A/AAAA addresses from a DNS response in parallel and return the lowest-latency IP to the client, improving real-world access speed.

---

## Phase 3 · MikroTik Deep Integration

On top of the existing one-way push, add the ability to pull RouterOS address lists as an OxiDNS data source and to actively push local IP sets to RouterOS, enabling bidirectional data integration between DNS policy and RouterOS.

---

## Phase 4 · OpenWrt Support

Bring a native install experience to OpenWrt users on par with the existing Debian package: one-command install via opkg, automatic service management, and system-integrated updates — no manual binary deployment required.

---

## Phase 5 · WebUI and Metrics Improvements

Add WebUI management interfaces for each new plugin, expand Prometheus metric coverage, and improve overall observability and operational experience.

---

<div style={{borderLeft: '4px solid var(--ifm-color-primary)', background: 'rgba(15, 118, 110, 0.06)', borderRadius: '0 12px 12px 0', padding: '0.9rem 1.2rem', marginTop: '2rem'}}>
  <p style={{margin: 0, lineHeight: 1.75}}><strong>Long-term direction: plugin ecosystem</strong></p>
  <ul style={{margin: '0.5rem 0 0', paddingLeft: '1.25rem', lineHeight: 1.75}}>
    <li><strong>WebAssembly plugins</strong>: Explore WASM-based third-party plugins so developers can write and distribute plugins in any language without modifying OxiDNS, with sandboxing included by default.</li>
    <li><strong>Dynamic library plugins</strong>: Explore native plugin loading via shared libraries (.so / .dylib) for scenarios with the highest performance requirements, allowing developers to compile and distribute plugins independently and have OxiDNS load them at runtime.</li>
  </ul>
</div>
