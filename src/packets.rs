use std::fmt;

use anyhow::Result;

#[derive(Debug, PartialEq, Eq)]
pub enum Packet {
    Relay(RelayPacket),
    Lora(Vec<u8>),
}

impl Packet {
    pub fn from_slice(b: &[u8]) -> Result<Self> {
        if b.is_empty() {
            return Err(anyhow!("Input is empty"));
        }

        // Check for proprietary "111" bits prefix.
        if b[0] & 0xe0 == 0xe0 {
            Ok(Packet::Relay(RelayPacket::from_slice(b)?))
        } else {
            Ok(Packet::Lora(b.to_vec()))
        }
    }

    pub fn to_vec(&self) -> Result<Vec<u8>> {
        match self {
            Packet::Relay(v) => v.to_vec(),
            Packet::Lora(v) => Ok(v.clone()),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RelayPacket {
    pub mhdr: MHDR,
    pub payload: Payload,
}

impl RelayPacket {
    pub fn from_slice(b: &[u8]) -> Result<Self> {
        if b.is_empty() {
            return Err(anyhow!("Input is empty"));
        }

        let mhdr = MHDR::from_byte(b[0])?;

        Ok(RelayPacket {
            payload: match mhdr.payload_type {
                PayloadType::Uplink => Payload::Uplink(UplinkPayload::from_slice(&b[1..])?),
                PayloadType::Downlink => Payload::Downlink(DownlinkPayload::from_slice(&b[1..])?),
            },
            mhdr,
        })
    }

    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut b = vec![self.mhdr.to_byte()?];
        b.extend_from_slice(&match &self.payload {
            Payload::Uplink(v) => v.to_vec()?,
            Payload::Downlink(v) => v.to_vec()?,
        });
        Ok(b)
    }
}

impl fmt::Display for RelayPacket {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.payload {
            Payload::Uplink(v) => write!(
                f,
                "[{:?} hop_count: {}, uplink_id: {}, relay_id: {}]",
                self.mhdr.payload_type,
                self.mhdr.hop_count,
                v.metadata.uplink_id,
                hex::encode(v.relay_id),
            ),
            Payload::Downlink(v) => write!(
                f,
                "[{:?} hop_count: {}, uplink_id: {}, relay_id: {}]",
                self.mhdr.payload_type,
                self.mhdr.hop_count,
                v.metadata.uplink_id,
                hex::encode(v.relay_id)
            ),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MHDR {
    pub payload_type: PayloadType,
    pub hop_count: u8, // 000 = 1, ... 111 = 8
}

impl MHDR {
    pub fn from_byte(b: u8) -> Result<Self> {
        if (b >> 5) != 0x07 {
            return Err(anyhow!("Invalid MType"));
        }

        Ok(MHDR {
            payload_type: PayloadType::from_byte((b >> 3) & 0x03)?,
            hop_count: (b & 0x07) + 1,
        })
    }

    pub fn to_byte(&self) -> Result<u8> {
        if self.hop_count == 0 {
            return Err(anyhow!("Min hop_count is 1"));
        }

        if self.hop_count > 8 {
            return Err(anyhow!("Max hop_count is 8"));
        }

        Ok(0x07 << 5 | self.payload_type.to_byte() << 3 | (self.hop_count - 1))
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PayloadType {
    Uplink,
    Downlink,
}

impl PayloadType {
    pub fn from_byte(b: u8) -> Result<Self> {
        Ok(match b {
            0x00 => PayloadType::Uplink,
            0x01 => PayloadType::Downlink,
            _ => return Err(anyhow!("Unexpected PayloadType: {}", b)),
        })
    }

    pub fn to_byte(&self) -> u8 {
        match self {
            PayloadType::Uplink => 0x00,
            PayloadType::Downlink => 0x01,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Payload {
    Uplink(UplinkPayload),
    Downlink(DownlinkPayload),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UplinkPayload {
    pub metadata: UplinkMetadata,
    pub relay_id: [u8; 4],
    pub phy_payload: Vec<u8>,
}

impl UplinkPayload {
    pub fn from_slice(b: &[u8]) -> Result<UplinkPayload> {
        if b.len() < 9 {
            return Err(anyhow!("At least 9 bytes are expected"));
        }

        let mut md = [0; 5];
        let mut gw_id = [0; 4];
        md.copy_from_slice(&b[0..5]);
        gw_id.copy_from_slice(&b[5..9]);

        Ok(UplinkPayload {
            metadata: UplinkMetadata::from_bytes(md),
            relay_id: gw_id,
            phy_payload: b[9..].to_vec(),
        })
    }

    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut b = self.metadata.to_bytes()?.to_vec();
        b.extend_from_slice(&self.relay_id);
        b.extend_from_slice(&self.phy_payload);
        Ok(b)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UplinkMetadata {
    pub uplink_id: u16,
    pub dr: u8,
    pub rssi: i16,
    pub snr: i8,
    pub channel: u8,
}

impl UplinkMetadata {
    pub fn from_bytes(b: [u8; 5]) -> Self {
        let snr = b[3] & 0x3f;
        let snr = if snr > 31 {
            (snr as i8) - 64
        } else {
            snr as i8
        };

        UplinkMetadata {
            uplink_id: u16::from_be_bytes([b[0], b[1]]) >> 4,
            dr: b[1] & 0x0f,
            rssi: -(b[2] as i16),
            snr,
            channel: b[4],
        }
    }

    pub fn to_bytes(&self) -> Result<[u8; 5]> {
        if self.uplink_id > 4095 {
            return Err(anyhow!("Max uplink_id value is 4095"));
        }

        if self.dr > 15 {
            return Err(anyhow!("Max dr value is 15"));
        }

        if self.rssi > 0 {
            return Err(anyhow!("Max rssi value is 0"));
        }

        if self.rssi < -255 {
            return Err(anyhow!("Min rssi value is -255"));
        }

        if self.snr < -32 {
            return Err(anyhow!("Min snr value is -32"));
        }
        if self.snr > 31 {
            return Err(anyhow!("Max snr value is 31"));
        }

        let uplink_id_b = (self.uplink_id << 4).to_be_bytes();

        Ok([
            uplink_id_b[0],
            uplink_id_b[1] | self.dr,
            -self.rssi as u8,
            if self.snr < 0 {
                (self.snr + 64) as u8
            } else {
                self.snr as u8
            },
            self.channel,
        ])
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DownlinkPayload {
    pub metadata: DownlinkMetadata,
    pub relay_id: [u8; 4],
    pub phy_payload: Vec<u8>,
}

impl DownlinkPayload {
    pub fn from_slice(b: &[u8]) -> Result<Self> {
        if b.len() < 10 {
            return Err(anyhow!("At least 10 bytes are expected"));
        }

        let mut md = [0; 6];
        let mut gw_id = [0; 4];
        md.copy_from_slice(&b[0..6]);
        gw_id.copy_from_slice(&b[6..10]);

        Ok(DownlinkPayload {
            metadata: DownlinkMetadata::from_bytes(md),
            relay_id: gw_id,
            phy_payload: b[10..].to_vec(),
        })
    }

    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut b = self.metadata.to_bytes()?.to_vec();
        b.extend_from_slice(&self.relay_id);
        b.extend_from_slice(&self.phy_payload);
        Ok(b)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DownlinkMetadata {
    pub uplink_id: u16,
    pub dr: u8,
    pub frequency: u32,
    pub tx_power: u8,
    pub delay: u8,
}

impl DownlinkMetadata {
    pub fn from_bytes(b: [u8; 6]) -> Self {
        DownlinkMetadata {
            uplink_id: u16::from_be_bytes([b[0], b[1]]) >> 4,
            dr: b[1] & 0x0f,
            frequency: decode_freq(&b[2..5]).unwrap(),
            tx_power: (b[5] & 0xf0) >> 4,
            delay: (b[5] & 0x0f) + 1,
        }
    }

    pub fn to_bytes(&self) -> Result<[u8; 6]> {
        if self.uplink_id > 4095 {
            return Err(anyhow!("Max uplink_id value is 4095"));
        }

        if self.dr > 15 {
            return Err(anyhow!("Max dr value is 15"));
        }

        if self.delay < 1 {
            return Err(anyhow!("Min delay value is 1"));
        }

        if self.tx_power > 15 {
            return Err(anyhow!("Max tx_power value is 15"));
        }

        if self.delay > 16 {
            return Err(anyhow!("Max delay value is 16"));
        }

        let uplink_id_b = (self.uplink_id << 4).to_be_bytes();
        let freq_b = encode_freq(self.frequency)?;

        Ok([
            uplink_id_b[0],
            uplink_id_b[1] | self.dr,
            freq_b[0],
            freq_b[1],
            freq_b[2],
            (self.tx_power << 4) | (self.delay - 1),
        ])
    }
}

pub fn encode_freq(freq: u32) -> Result<[u8; 3]> {
    let mut freq = freq;
    // Support LoRaWAN 2.4GHz, in which case the stepping is 200Hz:
    // See Frequency Encoding in MAC Commands
    // https://lora-developers.semtech.com/documentation/tech-papers-and-guides/physical-layer-proposal-2.4ghz/
    if freq >= 2400000000 {
        freq /= 2;
    }

    if freq / 100 >= (1 << 24) {
        return Err(anyhow!("Max frequency value is 2^24 - 1"));
    }
    if freq % 100 != 0 {
        return Err(anyhow!("Frequency must be multiple of 100"));
    }

    let mut b = [0; 3];
    b[0..3].copy_from_slice(&(freq / 100).to_be_bytes()[1..4]);
    Ok(b)
}

pub fn decode_freq(b: &[u8]) -> Result<u32> {
    if b.len() != 3 {
        return Err(anyhow!("3 bytes expected for frequency"));
    }
    let mut freq_b: [u8; 4] = [0; 4];
    freq_b[1..4].copy_from_slice(&b[0..3]);
    let mut freq = u32::from_be_bytes(freq_b);

    if freq >= 12000000 {
        // 2.4GHz frequency
        freq *= 200
    } else {
        freq *= 100
    }

    Ok(freq)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_mhdr_from_byte() {
        struct Test {
            name: String,
            byte: u8,
            expected_mhdr: Option<MHDR>,
            expected_error: Option<String>,
        }

        let tests = vec![
            Test {
                name: "uplink + hop count 3".to_string(),
                byte: 0xe2,
                expected_mhdr: Some(MHDR {
                    payload_type: PayloadType::Uplink,
                    hop_count: 3,
                }),
                expected_error: None,
            },
            Test {
                name: "downlink + hop count 8".to_string(),
                byte: 0xef,
                expected_mhdr: Some(MHDR {
                    payload_type: PayloadType::Downlink,
                    hop_count: 8,
                }),
                expected_error: None,
            },
            Test {
                name: "invalid MType".to_string(),
                byte: 0x00,
                expected_mhdr: None,
                expected_error: Some("Invalid MType".into()),
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let res = MHDR::from_byte(tst.byte);

            if let Some(mhdr) = &tst.expected_mhdr {
                assert_eq!(mhdr, &res.unwrap());
            } else if let Some(err) = &tst.expected_error {
                assert_eq!(err.to_string(), res.unwrap_err().to_string());
            }
        }
    }

    #[test]
    fn test_mhdr_to_byte() {
        struct Test {
            name: String,
            mhdr: MHDR,
            expected_byte: Option<u8>,
            expected_error: Option<String>,
        }

        let tests = vec![
            Test {
                name: "uplink + hop count 3".to_string(),
                mhdr: MHDR {
                    payload_type: PayloadType::Uplink,
                    hop_count: 3,
                },
                expected_byte: Some(0xe2),
                expected_error: None,
            },
            Test {
                name: "downlink + hop count 8".to_string(),
                mhdr: MHDR {
                    payload_type: PayloadType::Downlink,
                    hop_count: 8,
                },
                expected_byte: Some(0xef),
                expected_error: None,
            },
            Test {
                name: "hop count exceeds max value".to_string(),
                mhdr: MHDR {
                    payload_type: PayloadType::Uplink,
                    hop_count: 9,
                },
                expected_byte: None,
                expected_error: Some("Max hop_count is 8".into()),
            },
            Test {
                name: "hop count is 0".to_string(),
                mhdr: MHDR {
                    payload_type: PayloadType::Uplink,
                    hop_count: 0,
                },
                expected_byte: None,
                expected_error: Some("Min hop_count is 1".into()),
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let res = tst.mhdr.to_byte();

            if let Some(b) = &tst.expected_byte {
                assert_eq!(b, &res.unwrap());
            } else if let Some(err) = &tst.expected_error {
                assert_eq!(err.to_string(), res.unwrap_err().to_string());
            }
        }
    }

    #[test]
    fn test_uplink_metadata_to_bytes() {
        struct Test {
            name: String,
            metadata: UplinkMetadata,
            expected_bytes: Option<[u8; 5]>,
            expected_error: Option<String>,
        }

        let tests = vec![
            Test {
                name: "Uplink ID exceeds max value".into(),
                metadata: UplinkMetadata {
                    uplink_id: 4096,
                    dr: 0,
                    rssi: 0,
                    snr: 0,
                    channel: 0,
                },
                expected_bytes: None,
                expected_error: Some("Max uplink_id value is 4095".into()),
            },
            Test {
                name: "DR exceeds max value".into(),
                metadata: UplinkMetadata {
                    uplink_id: 0,
                    dr: 16,
                    rssi: 0,
                    snr: 0,
                    channel: 0,
                },
                expected_bytes: None,
                expected_error: Some("Max dr value is 15".into()),
            },
            Test {
                name: "RSSI exceeds max value".into(),
                metadata: UplinkMetadata {
                    uplink_id: 0,
                    dr: 0,
                    rssi: 1,
                    snr: 0,
                    channel: 0,
                },
                expected_bytes: None,
                expected_error: Some("Max rssi value is 0".into()),
            },
            Test {
                name: "RSSI exceeds min value".into(),
                metadata: UplinkMetadata {
                    uplink_id: 0,
                    dr: 0,
                    rssi: -256,
                    snr: 0,
                    channel: 0,
                },
                expected_bytes: None,
                expected_error: Some("Min rssi value is -255".into()),
            },
            Test {
                name: "SNR exceeds max value".into(),
                metadata: UplinkMetadata {
                    uplink_id: 0,
                    dr: 0,
                    rssi: 0,
                    snr: 32,
                    channel: 0,
                },
                expected_bytes: None,
                expected_error: Some("Max snr value is 31".into()),
            },
            Test {
                name: "SNR exceeds min value".into(),
                metadata: UplinkMetadata {
                    uplink_id: 0,
                    dr: 0,
                    rssi: 0,
                    snr: -33,
                    channel: 0,
                },
                expected_bytes: None,
                expected_error: Some("Min snr value is -32".into()),
            },
            Test {
                name: "Uplink id: 1024, dr: 3, rssi: -120, snr: -12, channel: 64".into(),
                metadata: UplinkMetadata {
                    uplink_id: 1024,
                    dr: 3,
                    rssi: -120,
                    snr: -12,
                    channel: 64,
                },
                expected_bytes: Some([0x40, 0x03, 0x78, 0x34, 0x40]),
                expected_error: None,
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let res = tst.metadata.to_bytes();

            if let Some(b) = &tst.expected_bytes {
                assert_eq!(b, &res.unwrap());
            } else if let Some(err) = &tst.expected_error {
                assert_eq!(err.to_string(), res.unwrap_err().to_string());
            }
        }
    }

    #[test]
    fn test_uplink_metadata_from_bytes() {
        struct Test {
            name: String,
            bytes: [u8; 5],
            expected_metadata: UplinkMetadata,
        }

        let tests = vec![Test {
            name: "Uplink id: 1024, dr: 3, rssi: -120, snr: -12, channel: 64".into(),
            bytes: [0x40, 0x03, 0x78, 0x34, 0x40],
            expected_metadata: UplinkMetadata {
                uplink_id: 1024,
                dr: 3,
                rssi: -120,
                snr: -12,
                channel: 64,
            },
        }];

        for tst in &tests {
            println!("> {}", tst.name);
            let res = UplinkMetadata::from_bytes(tst.bytes);
            assert_eq!(res, tst.expected_metadata);
        }
    }

    #[test]
    fn test_uplink_payload_from_vec() {
        let b = vec![0x40, 0x03, 0x78, 0x34, 0x40, 0x01, 0x02, 0x03, 0x04, 0x05];
        let up_pl = UplinkPayload::from_slice(&b).unwrap();
        assert_eq!(
            UplinkPayload {
                metadata: UplinkMetadata {
                    uplink_id: 1024,
                    dr: 3,
                    rssi: -120,
                    snr: -12,
                    channel: 64,
                },
                relay_id: [0x01, 0x02, 0x03, 0x04],
                phy_payload: vec![0x05],
            },
            up_pl,
        );
    }

    #[test]
    fn test_uplink_payload_to_vec() {
        let up_pl = UplinkPayload {
            metadata: UplinkMetadata {
                uplink_id: 1024,
                dr: 3,
                rssi: -120,
                snr: -12,
                channel: 64,
            },
            relay_id: [0x01, 0x02, 0x03, 0x04],
            phy_payload: vec![0x05],
        };
        let b = up_pl.to_vec().unwrap();
        assert_eq!(
            vec![0x40, 0x03, 0x78, 0x34, 0x40, 0x01, 0x02, 0x03, 0x04, 0x05],
            b
        );
    }

    #[test]
    fn test_downlink_metadata_from_bytes() {
        struct Test {
            name: String,
            bytes: [u8; 6],
            expected_metadata: DownlinkMetadata,
        }

        let tests = vec![Test {
            name: "Uplink id: 1024, dr: 3, frequency: 868100000, delay: 16".into(),
            bytes: [0x40, 0x03, 0x84, 0x76, 0x28, 0xff],
            expected_metadata: DownlinkMetadata {
                uplink_id: 1024,
                dr: 3,
                frequency: 868100000,
                tx_power: 15,
                delay: 16,
            },
        }];

        for tst in &tests {
            println!("> {}", tst.name);
            let res = DownlinkMetadata::from_bytes(tst.bytes);
            assert_eq!(res, tst.expected_metadata);
        }
    }

    #[test]
    fn test_downlink_metadata_to_bytes() {
        struct Test {
            name: String,
            metadata: DownlinkMetadata,
            expected_bytes: Option<[u8; 6]>,
            expected_error: Option<String>,
        }

        let tests = vec![
            Test {
                name: "Uplink ID exceeds max value".into(),
                metadata: DownlinkMetadata {
                    uplink_id: 4096,
                    dr: 0,
                    frequency: 868100000,
                    tx_power: 0,
                    delay: 1,
                },
                expected_bytes: None,
                expected_error: Some("Max uplink_id value is 4095".into()),
            },
            Test {
                name: "DR exceeds max value".into(),
                metadata: DownlinkMetadata {
                    uplink_id: 0,
                    dr: 16,
                    frequency: 868100000,
                    tx_power: 0,
                    delay: 1,
                },
                expected_bytes: None,
                expected_error: Some("Max dr value is 15".into()),
            },
            Test {
                name: "Frequency not multiple of 100".into(),
                metadata: DownlinkMetadata {
                    uplink_id: 0,
                    dr: 0,
                    frequency: 868100001,
                    tx_power: 0,
                    delay: 1,
                },
                expected_bytes: None,
                expected_error: Some("Frequency must be multiple of 100".into()),
            },
            Test {
                name: "TX Power exceeds max value".into(),
                metadata: DownlinkMetadata {
                    uplink_id: 0,
                    dr: 0,
                    frequency: 868100000,
                    tx_power: 16,
                    delay: 1,
                },
                expected_bytes: None,
                expected_error: Some("Max tx_power value is 15".into()),
            },
            Test {
                name: "Delay exceeds max value".into(),
                metadata: DownlinkMetadata {
                    uplink_id: 0,
                    dr: 0,
                    frequency: 868100000,
                    tx_power: 0,
                    delay: 17,
                },
                expected_bytes: None,
                expected_error: Some("Max delay value is 16".into()),
            },
            Test {
                name: "Uplink id: 1024, dr: 3, frequency: 868100000, tx_power: 15, delay: 16"
                    .into(),
                metadata: DownlinkMetadata {
                    uplink_id: 1024,
                    dr: 3,
                    frequency: 868100000,
                    tx_power: 15,
                    delay: 16,
                },
                expected_bytes: Some([0x40, 0x03, 0x84, 0x76, 0x28, 0xff]),
                expected_error: None,
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let res = tst.metadata.to_bytes();

            if let Some(b) = &tst.expected_bytes {
                assert_eq!(b, &res.unwrap());
            } else if let Some(err) = &tst.expected_error {
                assert_eq!(err.to_string(), res.unwrap_err().to_string());
            }
        }
    }

    #[test]
    fn test_downlink_payload_from_slice() {
        let b = vec![
            0x40, 0x03, 0x84, 0x76, 0x28, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05,
        ];
        let dn_pl = DownlinkPayload::from_slice(&b).unwrap();
        assert_eq!(
            DownlinkPayload {
                metadata: DownlinkMetadata {
                    uplink_id: 1024,
                    dr: 3,
                    frequency: 868100000,
                    tx_power: 15,
                    delay: 16,
                },
                relay_id: [0x01, 0x02, 0x03, 0x04],
                phy_payload: vec![0x05],
            },
            dn_pl,
        );
    }

    #[test]
    fn test_downlink_payload_to_vec() {
        let dn_pl = DownlinkPayload {
            metadata: DownlinkMetadata {
                uplink_id: 1024,
                dr: 3,
                frequency: 868100000,
                tx_power: 15,
                delay: 16,
            },
            relay_id: [0x01, 0x02, 0x03, 0x04],
            phy_payload: vec![0x05],
        };
        let b = dn_pl.to_vec().unwrap();
        assert_eq!(
            vec![0x40, 0x03, 0x84, 0x76, 0x28, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05,],
            b
        );
    }

    #[test]
    fn test_relay_packet_from_slice() {
        struct Test {
            name: String,
            bytes: Vec<u8>,
            expected_relay_packet: RelayPacket,
        }

        let tests = vec![
            Test {
                name: "uplink".into(),
                bytes: vec![
                    0xe2, 0x40, 0x03, 0x78, 0x34, 0x40, 0x01, 0x02, 0x03, 0x04, 0x05,
                ],
                expected_relay_packet: RelayPacket {
                    mhdr: MHDR {
                        payload_type: PayloadType::Uplink,
                        hop_count: 3,
                    },
                    payload: Payload::Uplink(UplinkPayload {
                        metadata: UplinkMetadata {
                            uplink_id: 1024,
                            dr: 3,
                            rssi: -120,
                            snr: -12,
                            channel: 64,
                        },
                        relay_id: [0x01, 0x02, 0x03, 0x04],
                        phy_payload: vec![0x05],
                    }),
                },
            },
            Test {
                name: "downlink".into(),
                bytes: vec![
                    0xef, 0x40, 0x03, 0x84, 0x76, 0x28, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05,
                ],
                expected_relay_packet: RelayPacket {
                    mhdr: MHDR {
                        payload_type: PayloadType::Downlink,
                        hop_count: 8,
                    },
                    payload: Payload::Downlink(DownlinkPayload {
                        metadata: DownlinkMetadata {
                            uplink_id: 1024,
                            dr: 3,
                            frequency: 868100000,
                            tx_power: 15,
                            delay: 16,
                        },
                        relay_id: [0x01, 0x02, 0x03, 0x04],
                        phy_payload: vec![0x05],
                    }),
                },
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let pl = RelayPacket::from_slice(&tst.bytes).unwrap();
            assert_eq!(tst.expected_relay_packet, pl);
        }
    }

    #[test]
    fn test_relay_packet_to_vec() {
        struct Test {
            name: String,
            relay_packet: RelayPacket,
            expected_bytes: Vec<u8>,
        }

        let tests = vec![
            Test {
                name: "uplink".into(),
                expected_bytes: vec![
                    0xe2, 0x40, 0x03, 0x78, 0x34, 0x40, 0x01, 0x02, 0x03, 0x04, 0x05,
                ],
                relay_packet: RelayPacket {
                    mhdr: MHDR {
                        payload_type: PayloadType::Uplink,
                        hop_count: 3,
                    },
                    payload: Payload::Uplink(UplinkPayload {
                        metadata: UplinkMetadata {
                            uplink_id: 1024,
                            dr: 3,
                            rssi: -120,
                            snr: -12,
                            channel: 64,
                        },
                        relay_id: [0x01, 0x02, 0x03, 0x04],
                        phy_payload: vec![0x05],
                    }),
                },
            },
            Test {
                name: "downlink".into(),
                expected_bytes: vec![
                    0xef, 0x40, 0x03, 0x84, 0x76, 0x28, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05,
                ],
                relay_packet: RelayPacket {
                    mhdr: MHDR {
                        payload_type: PayloadType::Downlink,
                        hop_count: 8,
                    },
                    payload: Payload::Downlink(DownlinkPayload {
                        metadata: DownlinkMetadata {
                            uplink_id: 1024,
                            dr: 3,
                            frequency: 868100000,
                            tx_power: 15,
                            delay: 16,
                        },
                        relay_id: [0x01, 0x02, 0x03, 0x04],
                        phy_payload: vec![0x05],
                    }),
                },
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let b = tst.relay_packet.to_vec().unwrap();
            assert_eq!(tst.expected_bytes, b);
        }
    }

    #[test]
    fn test_packet_from_slice() {
        struct Test {
            name: String,
            bytes: Vec<u8>,
            expected_packet: Packet,
        }

        let tests = vec![
            Test {
                name: "relay packet".into(),
                bytes: vec![
                    0xe2, 0x40, 0x03, 0x78, 0x34, 0x40, 0x01, 0x02, 0x03, 0x04, 0x05,
                ],
                expected_packet: Packet::Relay(RelayPacket {
                    mhdr: MHDR {
                        payload_type: PayloadType::Uplink,
                        hop_count: 3,
                    },
                    payload: Payload::Uplink(UplinkPayload {
                        metadata: UplinkMetadata {
                            uplink_id: 1024,
                            dr: 3,
                            rssi: -120,
                            snr: -12,
                            channel: 64,
                        },
                        relay_id: [0x01, 0x02, 0x03, 0x04],
                        phy_payload: vec![0x05],
                    }),
                }),
            },
            Test {
                name: "lora packet".into(),
                bytes: vec![0x01, 0x02, 0x03],
                expected_packet: Packet::Lora(vec![0x01, 0x02, 0x03]),
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let pkt = Packet::from_slice(&tst.bytes).unwrap();
            assert_eq!(tst.expected_packet, pkt);
        }
    }

    #[test]
    fn test_packet_to_vec() {
        struct Test {
            name: String,
            expected_bytes: Vec<u8>,
            packet: Packet,
        }

        let tests = vec![
            Test {
                name: "relay packet".into(),
                expected_bytes: vec![
                    0xe2, 0x40, 0x03, 0x78, 0x34, 0x40, 0x01, 0x02, 0x03, 0x04, 0x05,
                ],
                packet: Packet::Relay(RelayPacket {
                    mhdr: MHDR {
                        payload_type: PayloadType::Uplink,
                        hop_count: 3,
                    },
                    payload: Payload::Uplink(UplinkPayload {
                        metadata: UplinkMetadata {
                            uplink_id: 1024,
                            dr: 3,
                            rssi: -120,
                            snr: -12,
                            channel: 64,
                        },
                        relay_id: [0x01, 0x02, 0x03, 0x04],
                        phy_payload: vec![0x05],
                    }),
                }),
            },
            Test {
                name: "lora packet".into(),
                expected_bytes: vec![0x01, 0x02, 0x03],
                packet: Packet::Lora(vec![0x01, 0x02, 0x03]),
            },
        ];

        for tst in &tests {
            println!("> {}", tst.name);
            let b = tst.packet.to_vec().unwrap();
            assert_eq!(tst.expected_bytes, b);
        }
    }
}
