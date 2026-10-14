//! `mesh-sim`: delivery smoke test (section 21: 50 nodes, 1,000 messages,
//! ≥ 99 % delivered within 10 simulated minutes).
//!
//! Usage: mesh-sim [nodes] [messages] [area_m] [seed]

use mesh_api::*;
use mesh_simulator::*;
use std::collections::HashMap;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let nodes: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(50);
    let messages: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let area: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(150.0);
    let seed: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(42);
    let started = Instant::now();
    let r = run(nodes, messages, area, seed);
    println!("{}", r.report());
    println!("wall time: {:.1}s", started.elapsed().as_secs_f64());
    if r.within_10min < 0.99 {
        std::process::exit(1);
    }
}

pub struct Result {
    pub nodes: usize,
    pub sent: usize,
    pub delivered: usize,
    pub within_10min: f64,
    pub median_s: f64,
    pub p95_s: f64,
    pub frames: u64,
    pub bytes: u64,
}

impl Result {
    fn report(&self) -> String {
        format!(
            "nodes={} sent={} delivered={} within_10min={:.2}% median={:.1}s p95={:.1}s frames={} MB={:.1}",
            self.nodes,
            self.sent,
            self.delivered,
            self.within_10min * 100.0,
            self.median_s,
            self.p95_s,
            self.frames,
            self.bytes as f64 / 1e6
        )
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn f(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }
}

pub fn run(n: usize, messages: usize, area: f64, seed: u64) -> Result {
    let mut w = World::new(1_790_000_000_000, seed);
    w.collect_events = false;
    w.area = (area, area);
    let mut rng = Lcg(seed);
    for i in 0..n {
        w.add_user(&format!("u{i}"), (rng.f() * area, rng.f() * area));
    }
    // Pair i with i+1 and i+7 at isolated pairing spots, all in parallel.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..n {
        pairs.push((i, (i + 1) % n));
        if n > 8 {
            pairs.push((i, (i + 7) % n));
        }
    }
    let homes: Vec<(f64, f64)> = w.nodes.iter().map(|x| x.pos).collect();
    let mut remaining = pairs.clone();
    while !remaining.is_empty() {
        let mut busy = vec![false; n];
        let mut batch = Vec::new();
        let mut rest = Vec::new();
        for &(a, b) in &remaining {
            if busy[a] || busy[b] {
                rest.push((a, b));
                continue;
            }
            busy[a] = true;
            busy[b] = true;
            let spot = (10_000.0 + batch.len() as f64 * 500.0, 10_000.0);
            w.nodes[a].pos = spot;
            w.nodes[b].pos = (spot.0 + 5.0, spot.1);
            batch.push((a, b));
        }
        w.area = (1e9, 1e9);
        w.run_ms(1_500);
        for &(a, b) in &batch {
            let qr = w.nodes[b].engine.pairing_qr().unwrap();
            w.nodes[a].engine.pair_from_qr(qr).unwrap();
        }
        w.run_until(30_000, |w| {
            batch
                .iter()
                .all(|&(a, b)| w.conv_with(a, b).is_some() && w.conv_with(b, a).is_some())
        });
        w.run_ms(3_000);
        for &(a, b) in &batch {
            w.nodes[a].pos = homes[a];
            w.nodes[b].pos = homes[b];
        }
        remaining = rest;
    }
    w.area = (area, area);
    // Walking speed 1.4 m/s in random directions.
    for node in &mut w.nodes {
        let th = rng.f() * std::f64::consts::TAU;
        node.vel = (1.4 * th.cos(), 1.4 * th.sin());
    }
    let paired: Vec<(usize, usize)> = pairs
        .iter()
        .copied()
        .filter(|&(a, b)| w.conv_with(a, b).is_some() && w.conv_with(b, a).is_some())
        .collect();
    eprintln!("paired {}/{} pairs", paired.len(), pairs.len());
    w.run_ms(10_000);
    // Send messages spread over 2 minutes.
    let start = w.now();
    let mut schedule: Vec<(u64, usize, usize, String)> = (0..messages)
        .map(|k| {
            let (a, b) = paired[rng.next() as usize % paired.len()];
            let (s, r) = if rng.next().is_multiple_of(2) {
                (a, b)
            } else {
                (b, a)
            };
            (start + rng.next() % 120_000, s, r, format!("m{k}"))
        })
        .collect();
    schedule.sort_by_key(|x| x.0);
    let mut sent_at: HashMap<String, (u64, usize, usize)> = HashMap::new();
    let mut delivered_at: HashMap<String, u64> = HashMap::new();
    let mut next = 0;
    let end = start + 120_000 + 10 * 60_000;
    let mut last_check = 0;
    while w.now() < end {
        while next < schedule.len() && schedule[next].0 <= w.now() {
            let (_, s, r, text) = schedule[next].clone();
            let conv = w.conv_with(s, r).unwrap();
            if w.nodes[s]
                .engine
                .send_text(conv, text.clone(), SendOptions::default())
                .is_ok()
            {
                sent_at.insert(text, (w.now(), s, r));
            }
            next += 1;
        }
        w.step();
        if w.now() - last_check >= 5_000 {
            last_check = w.now();
            let pending: Vec<(String, usize, usize)> = sent_at
                .iter()
                .filter(|(t, _)| !delivered_at.contains_key(*t))
                .map(|(t, v)| (t.clone(), v.1, v.2))
                .collect();
            let mut by_node: HashMap<(usize, usize), Vec<String>> = HashMap::new();
            for (t, s, r) in pending {
                by_node.entry((r, s)).or_default().push(t);
            }
            for ((r, s), texts) in by_node {
                let Some(conv) = w.conv_with(r, s) else {
                    continue;
                };
                let msgs = w.nodes[r]
                    .engine
                    .messages(conv, None, 100_000)
                    .unwrap_or_default();
                for t in texts {
                    if msgs.iter().any(|m| m.text == t) {
                        delivered_at.insert(t, w.now());
                    }
                }
            }
        }
    }
    let cf: u32 = w
        .nodes
        .iter()
        .map(|x| x.engine.network_status().crypto_failures)
        .sum();
    let relay: u32 = w
        .nodes
        .iter()
        .map(|x| x.engine.network_status().relay_envelopes)
        .sum();
    let neigh: u32 = w
        .nodes
        .iter()
        .map(|x| x.engine.network_status().neighbours)
        .sum();
    eprintln!(
        "crypto_failures={cf} relay_envelopes_total={relay} avg_neighbours={:.1}",
        neigh as f64 / n as f64
    );
    let mut states: HashMap<String, usize> = HashMap::new();
    for (t, (_, s, r)) in sent_at.iter().take(2000) {
        let conv = w.conv_with(*s, *r).unwrap();
        let st = w.nodes[*s]
            .engine
            .messages(conv, None, 100_000)
            .unwrap()
            .into_iter()
            .find(|m| &m.text == t)
            .map(|m| format!("{:?}", m.state))
            .unwrap_or("missing".into());
        *states
            .entry(format!(
                "{}{}",
                st,
                if delivered_at.contains_key(t) {
                    "+rx"
                } else {
                    ""
                }
            ))
            .or_default() += 1;
    }
    eprintln!("sender states: {states:?}");
    let mut lat: Vec<f64> = delivered_at
        .iter()
        .map(|(t, d)| (*d - sent_at[t].0) as f64 / 1000.0)
        .collect();
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let within = lat.iter().filter(|l| **l <= 600.0).count();
    let (frames, bytes) = w.traffic();
    Result {
        nodes: n,
        sent: sent_at.len(),
        delivered: lat.len(),
        within_10min: within as f64 / sent_at.len().max(1) as f64,
        median_s: lat.get(lat.len() / 2).copied().unwrap_or(f64::NAN),
        p95_s: lat.get(lat.len() * 95 / 100).copied().unwrap_or(f64::NAN),
        frames,
        bytes,
    }
}
