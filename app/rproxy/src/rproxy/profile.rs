use std::fmt::Debug;

#[derive(Default)]
pub struct ReverseProxyProfile {
    active_conns: u64,
    closed_conns: u64,
    prohibited_conns: u64,
    forwarding_datas: u64,
    backwarding_datas: u64,
}

impl ReverseProxyProfile {
    // pub fn update(&mut self, sample: Sample) {
    //     for event in sample.events_update {
    //         match event {
    //             ProfileEvent::Connect(_) => {
    //                 self.active_conns += 1;
    //             }
    //             ProfileEvent::Disconnect(_) => {
    //                 self.active_conns -= 1;
    //                 self.closed_conns += 1;
    //             }
    //             ProfileEvent::Prohibited(_) => {
    //                 self.prohibited_conns += 1;
    //             }
    //             ProfileEvent::OpenStream(_) => {
    //                 self.active_conns += 1;
    //             }
    //             ProfileEvent::CloseStream(_) => {
    //                 self.active_conns -= 1;
    //                 self.closed_conns += 1;
    //             }
    //             ProfileEvent::Transport(transport) => {
    //                 self.backwarding_datas += transport.backwarding_data;
    //                 self.forwarding_datas += transport.forwarding_data;
    //             }
    //         }
    //     }
    // }
}

impl Debug for ReverseProxyProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ac={}, cc={}, pc={}, fd={}, bd={}",
            self.active_conns,
            self.closed_conns,
            self.prohibited_conns,
            self.forwarding_datas,
            self.backwarding_datas
        )
    }
}
