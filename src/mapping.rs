use pallas::ledger::alonzo::{
    self as alonzo, crypto::hash_transaction, AuxiliaryData, Block, Certificate,
    InstantaneousRewardSource, InstantaneousRewardTarget, Metadata, Metadatum, Relay,
    TransactionOutput, Value,
};

use crate::framework::{EventContext, EventData, EventSource, EventWriter, StakeCredential};

use crate::framework::Error;

pub trait ToHex {
    fn to_hex(&self) -> String;
}

impl ToHex for Vec<u8> {
    fn to_hex(&self) -> String {
        hex::encode(self)
    }
}

impl From<&alonzo::StakeCredential> for StakeCredential {
    fn from(other: &alonzo::StakeCredential) -> Self {
        match other {
            alonzo::StakeCredential::AddrKeyhash(x) => StakeCredential::AddrKeyhash(x.to_hex()),
            alonzo::StakeCredential::Scripthash(x) => StakeCredential::Scripthash(x.to_hex()),
        }
    }
}

fn ip_string_from_bytes(bytes: &[u8]) -> String {
    format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])
}

fn relay_to_string(relay: &Relay) -> String {
    match relay {
        Relay::SingleHostAddr(port, ipv4, ipv6) => {
            let ip = match (ipv6, ipv4) {
                (None, None) => "".to_string(),
                (_, Some(x)) => ip_string_from_bytes(x.as_ref()),
                (Some(x), _) => ip_string_from_bytes(x.as_ref()),
            };

            match port {
                Some(port) => format!("{}:{}", ip, port),
                None => ip,
            }
        }
        Relay::SingleHostName(port, host) => match port {
            Some(port) => format!("{}:{}", host, port),
            None => host.clone(),
        },
        Relay::MultiHostName(host) => host.clone(),
    }
}

impl EventSource for Certificate {
    fn write_events(&self, writer: &EventWriter) -> Result<(), Error> {
        let event = match self {
            Certificate::StakeRegistration(credential) => EventData::StakeRegistration {
                credential: credential.into(),
            },
            Certificate::StakeDeregistration(credential) => EventData::StakeDeregistration {
                credential: credential.into(),
            },
            Certificate::StakeDelegation(credential, pool) => EventData::StakeDelegation {
                credential: credential.into(),
                pool_hash: pool.to_hex(),
            },
            Certificate::PoolRegistration {
                operator,
                vrf_keyhash,
                pledge,
                cost,
                margin,
                reward_account,
                pool_owners,
                relays,
                pool_metadata,
            } => EventData::PoolRegistration {
                operator: operator.to_hex(),
                vrf_keyhash: vrf_keyhash.to_hex(),
                pledge: *pledge,
                cost: *cost,
                margin: (margin.numerator as f64 / margin.denominator as f64),
                reward_account: reward_account.to_hex(),
                pool_owners: pool_owners.iter().map(|p| p.to_hex()).collect(),
                relays: relays.iter().map(relay_to_string).collect(),
                pool_metadata: pool_metadata.as_ref().map(|m| m.url.clone()),
            },
            Certificate::PoolRetirement(pool, epoch) => EventData::PoolRetirement {
                pool: pool.to_hex(),
                epoch: *epoch,
            },
            Certificate::MoveInstantaneousRewardsCert(move_) => {
                EventData::MoveInstantaneousRewardsCert {
                    from_reserves: matches!(move_.source, InstantaneousRewardSource::Reserves),
                    from_treasury: matches!(move_.source, InstantaneousRewardSource::Treasury),
                    to_stake_credentials: match &move_.target {
                        InstantaneousRewardTarget::StakeCredentials(creds) => {
                            let x = creds.iter().map(|(k, v)| (k.into(), *v)).collect();
                            Some(x)
                        }
                        _ => None,
                    },
                    to_other_pot: match move_.target {
                        InstantaneousRewardTarget::OtherAccountingPot(x) => Some(x),
                        _ => None,
                    },
                }
            }

            // TODO: not likely, leaving for later
            Certificate::GenesisKeyDelegation(..) => EventData::GenesisKeyDelegation,
        };

        writer.append(event)?;

        Ok(())
    }
}

fn metadatum_to_string(datum: &Metadatum) -> String {
    match datum {
        Metadatum::Int(x) => x.to_string(),
        Metadatum::Bytes(x) => hex::encode::<&Vec<u8>>(x.as_ref()),
        Metadatum::Text(x) => x.to_owned(),
        Metadatum::Array(x) => x
            .iter()
            .map(|i| format!("{}, ", metadatum_to_string(i)))
            .collect(),
        Metadatum::Map(x) => x
            .iter()
            .map(|(key, val)| {
                format!(
                    "{}: {}, ",
                    metadatum_to_string(key),
                    metadatum_to_string(val)
                )
            })
            .collect(),
    }
}

impl EventSource for Metadata {
    fn write_events(&self, writer: &EventWriter) -> Result<(), Error> {
        for (level1_key, level1_data) in self.iter() {
            match level1_data {
                Metadatum::Map(level1_map) => {
                    for (level2_key, level2_data) in level1_map.iter() {
                        writer.append(EventData::Metadata {
                            key: metadatum_to_string(level1_key),
                            subkey: Some(metadatum_to_string(level2_key)),
                            value: Some(metadatum_to_string(level2_data)),
                        })?;
                    }
                }
                _ => {
                    writer.append(EventData::Metadata {
                        key: metadatum_to_string(level1_key),
                        subkey: None,
                        value: None,
                    })?;
                }
            }
        }

        Ok(())
    }
}

impl EventSource for AuxiliaryData {
    fn write_events(&self, writer: &EventWriter) -> Result<(), Error> {
        match self {
            AuxiliaryData::Alonzo(data) => {
                if let Some(metadata) = &data.metadata {
                    metadata.write_events(writer)?;
                }

                for _native in data.native_scripts.iter() {
                    writer.append(EventData::NewNativeScript)?;
                }

                for plutus in data.plutus_scripts.iter() {
                    writer.append(EventData::NewPlutusScript {
                        data: plutus.to_hex(),
                    })?;
                }
            }
            AuxiliaryData::Shelley(data) => {
                data.write_events(writer)?;
            }
            _ => log::warn!("ShelleyMa auxiliary data, not sure what to do"),
        }

        Ok(())
    }
}

impl EventSource for TransactionOutput {
    fn write_events(&self, writer: &EventWriter) -> Result<(), Error> {
        writer.append(EventData::TxOutput {
            address: self.address.to_hex(),
            amount: match self.amount {
                Value::Coin(x) => x,
                Value::Multiasset(x, _) => x,
            },
        })?;

        if let Value::Multiasset(_, assets) = &self.amount {
            for (policy, assets) in assets.iter() {
                for (asset, amount) in assets.iter() {
                    writer.append(EventData::OutputAsset {
                        policy: policy.to_hex(),
                        asset: asset.to_hex(),
                        amount: *amount,
                    })?;
                }
            }
        }

        Ok(())
    }
}

impl EventSource for Block {
    fn write_events(&self, writer: &EventWriter) -> Result<(), Error> {
        let writer = writer.child_writer(EventContext {
            block_number: Some(self.header.header_body.block_number),
            slot: Some(self.header.header_body.slot),
            ..EventContext::default()
        });

        writer.append(EventData::Block {
            body_size: self.header.header_body.block_body_size as usize,
            issuer_vkey: self.header.header_body.issuer_vkey.to_hex(),
        })?;

        for (idx, tx) in self.transaction_bodies.iter().enumerate() {
            let tx_hash = match hash_transaction(tx) {
                Ok(h) => Some(hex::encode(h)),
                Err(err) => {
                    log::warn!("error hashing transaction: {:?}", err);
                    None
                }
            };

            let writer = writer.child_writer(EventContext {
                tx_idx: Some(idx),
                tx_hash: tx_hash.clone(),
                ..EventContext::default()
            });

            writer.append(EventData::Transaction {
                hash: tx_hash,
                fee: tx.fee,
                ttl: tx.ttl,
                validity_interval_start: tx.validity_interval_start,
            })?;

            if let Some(mint) = &tx.mint {
                for (policy, value) in mint.iter() {
                    for (asset, quantity) in value.iter() {
                        writer.append(EventData::Mint {
                            policy: policy.to_hex(),
                            asset: asset.to_hex(),
                            quantity: *quantity,
                        })?;
                    }
                }
            }

            if let Some(certs) = &tx.certificates {
                for cert in certs.iter() {
                    cert.write_events(&writer)?;
                }
            }

            if let Some(aux) = self.auxiliary_data_set.get(&(idx as u32)) {
                aux.write_events(&writer)?;
            };

            if let Some(witness) = self.transaction_witness_sets.get(idx) {
                if let Some(scripts) = &witness.plutus_script {
                    for script in scripts.iter() {
                        writer.append(EventData::PlutusScriptRef {
                            data: script.to_hex(),
                        })?;
                    }
                }
            }

            for (idx, input) in tx.inputs.iter().enumerate() {
                let writer = writer.child_writer(EventContext {
                    input_idx: Some(idx),
                    ..EventContext::default()
                });

                writer.append(EventData::TxInput {
                    tx_id: input.transaction_id.to_hex(),
                    index: input.index,
                })?;
            }

            for (idx, output) in tx.outputs.iter().enumerate() {
                let writer = writer.child_writer(EventContext {
                    input_idx: Some(idx),
                    ..EventContext::default()
                });

                output.write_events(&writer)?;
            }
        }

        Ok(())
    }
}